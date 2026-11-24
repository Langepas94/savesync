//! Бэкенд Google Drive (plan §2.1): Drive API v3.
//!
//! Аутентификация — OAuth access token в конфиге (`backends.google.token`;
//! обновление refresh-токена — TODO при получении client id/secret, см.
//! plan §6 «OAuth»). Данные живут в скрытой области `appDataFolder`:
//! `SaveSync/index.json`, `SaveSync/saves/<game_key>/<version_id>.zip`.
//!
//! Drive не имеет директорий как путей — резолвим имя → file id по
//! родителю, с кешем на время жизни бэкенда.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read};
use std::time::Duration;

const DEFAULT_BASE: &str = "https://www.googleapis.com";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const ROOT_SPACE: &str = "appDataFolder";

/// Канал синхронизации через Google Drive.
pub struct GoogleDrive {
    token: String,
    base_url: String,
    agent: ureq::Agent,
    /// Кеш резолва путей: "saves/elden-ring" -> file id.
    path_cache: RefCell<HashMap<String, String>>,
}

impl GoogleDrive {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(token, DEFAULT_BASE)
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build();
        GoogleDrive {
            token: token.into(),
            base_url: base_url.into(),
            agent,
            path_cache: RefCell::new(HashMap::new()),
        }
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        req.set("Authorization", &format!("Bearer {}", self.token))
    }

    fn api_error(&self, op: &str, err: ureq::Error) -> io::Error {
        let kind = match &err {
            ureq::Error::Status(401 | 403, _) => io::ErrorKind::PermissionDenied,
            ureq::Error::Status(404, _) => io::ErrorKind::NotFound,
            ureq::Error::Status(_, _) => io::ErrorKind::Other,
            ureq::Error::Transport(_) => io::ErrorKind::ConnectionAborted,
        };
        io::Error::new(kind, format!("google drive {op}: {err}"))
    }

    /// Список детей папки: (id, name, is_folder).
    fn list_children(&self, parent_id: &str) -> io::Result<Vec<(String, String, bool)>> {
        let q = format!("'{parent_id}' in parents and trashed = false");
        let resp = self
            .auth(self.agent.get(&format!("{}/drive/v3/files", self.base_url)))
            .query("q", &q)
            .query("spaces", ROOT_SPACE)
            .query("fields", "files(id,name,mimeType)")
            .query("pageSize", "1000")
            .call()
            .map_err(|e| self.api_error("list", e))?;
        let json: serde_json::Value = resp
            .into_json()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut out = Vec::new();
        if let Some(files) = json.get("files").and_then(|f| f.as_array()) {
            for f in files {
                let id = f.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                let name = f.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                let is_dir = f.get("mimeType").and_then(|v| v.as_str()) == Some(FOLDER_MIME);
                if !id.is_empty() && !name.is_empty() {
                    out.push((id.to_string(), name.to_string(), is_dir));
                }
            }
        }
        Ok(out)
    }

    /// Создать файл/папку по имени в родителе. Возвращает id.
    fn create(&self, name: &str, parent_id: &str, is_folder: bool) -> io::Result<String> {
        let body = serde_json::json!({
            "name": name,
            "parents": [parent_id],
            "mimeType": if is_folder { FOLDER_MIME } else { "application/octet-stream" },
        });
        let resp = self
            .auth(
                self.agent
                    .post(&format!("{}/drive/v3/files", self.base_url)),
            )
            .set("Content-Type", "application/json")
            .query("spaces", ROOT_SPACE)
            .send_json(body)
            .map_err(|e| self.api_error("create", e))?;
        let json: serde_json::Value = resp
            .into_json()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        json.get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no id in create response"))
    }

    /// Id сегмента пути внутри родителя; создаёт папку, если нужно.
    fn child_id(
        &self,
        parent_id: &str,
        name: &str,
        create_folder: bool,
    ) -> io::Result<Option<String>> {
        for (id, child, _is_dir) in self.list_children(parent_id)? {
            if child == name {
                return Ok(Some(id));
            }
        }
        if create_folder {
            return Ok(Some(self.create(name, parent_id, true)?));
        }
        Ok(None)
    }

    /// Резолв пути вида `saves/<game>` в id; create=true создаёт папки.
    /// Пустой путь — корневая папка SaveSync.
    fn resolve(&self, rel: &str, create: bool) -> io::Result<Option<String>> {
        if let Some(id) = self.path_cache.borrow().get(rel) {
            return Ok(Some(id.clone()));
        }
        let cached_root = self.path_cache.borrow().get("").cloned();
        let mut parent = match cached_root {
            Some(id) => id,
            None => {
                let id = match self.child_id(ROOT_SPACE, "SaveSync", create)? {
                    Some(id) => id,
                    None => return Ok(None),
                };
                self.path_cache
                    .borrow_mut()
                    .insert(String::new(), id.clone());
                id
            }
        };
        if rel.is_empty() {
            return Ok(Some(parent));
        }
        for seg in rel.split('/') {
            let Some(id) = self.child_id(&parent, seg, create)? else {
                return Ok(None);
            };
            parent = id;
        }
        self.path_cache
            .borrow_mut()
            .insert(rel.to_string(), parent.clone());
        Ok(Some(parent))
    }

    /// id файла по пути (без создания).
    fn file_id(&self, rel: &str) -> io::Result<Option<String>> {
        let (dir, name) = match rel.rsplit_once('/') {
            Some((d, n)) => (d, n),
            None => ("", rel),
        };
        let Some(parent) = self.resolve(dir, false)? else {
            return Ok(None);
        };
        self.child_id(&parent, name, false)
    }

    fn upload(&self, rel: &str, data: &[u8]) -> io::Result<()> {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let parent = self
            .resolve(dir, true)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no parent folder"))?;
        if let Some(id) = self.child_id(&parent, name, false)? {
            // Перезапись содержимого существующего файла.
            self.auth(
                self.agent
                    .patch(&format!("{}/upload/drive/v3/files/{id}", self.base_url)),
            )
            .query("uploadType", "media")
            .set("Content-Type", "application/octet-stream")
            .send_bytes(data)
            .map_err(|e| self.api_error("upload", e))?;
        } else {
            // Создание: метаданные, затем контент.
            let id = self.create(name, &parent, false)?;
            self.auth(
                self.agent
                    .patch(&format!("{}/upload/drive/v3/files/{id}", self.base_url)),
            )
            .query("uploadType", "media")
            .set("Content-Type", "application/octet-stream")
            .send_bytes(data)
            .map_err(|e| self.api_error("upload", e))?;
        }
        Ok(())
    }

    fn download(&self, rel: &str) -> io::Result<Vec<u8>> {
        let Some(id) = self.file_id(rel)? else {
            return Err(io::Error::new(io::ErrorKind::NotFound, rel.to_string()));
        };
        let resp = self
            .auth(
                self.agent
                    .get(&format!("{}/drive/v3/files/{id}", self.base_url)),
            )
            .query("alt", "media")
            .call()
            .map_err(|e| self.api_error("download", e))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(512 * 1024 * 1024)
            .read_to_end(&mut buf)?;
        Ok(buf)
    }
}

impl crate::backend::Backend for GoogleDrive {
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()> {
        self.upload(&format!("saves/{game_key}/{version_id}.zip"), data)
    }

    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>> {
        self.download(&format!("saves/{game_key}/{version_id}.zip"))
    }

    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>> {
        let Some(dir_id) = self.resolve(&format!("saves/{game_key}"), false)? else {
            return Ok(Vec::new());
        };
        let mut out: Vec<String> = self
            .list_children(&dir_id)?
            .into_iter()
            .filter(|(_, _, is_dir)| !is_dir)
            .filter_map(|(_, name, _)| name.strip_suffix(".zip").map(str::to_string))
            .collect();
        out.sort();
        Ok(out)
    }

    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        match self.download("index.json") {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        self.upload("index.json", data)
    }

    fn name(&self) -> &'static str {
        "google-drive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    /// id -> (name, parent_id, mime, content) — вынесено из-за сложности типа.
    type FileRecord = (String, String, String, Vec<u8>);

    /// Mock Drive API v3: файлы в памяти, id = "id<N>".
    struct MockDrive {
        files: Mutex<HashMap<String, FileRecord>>,
        next_id: Mutex<u64>,
        auth_header: Mutex<Option<String>>,
    }

    impl MockDrive {
        fn insert(&self, name: &str, parent: &str, mime: &str, content: Vec<u8>) -> String {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            let id = format!("id{n}");
            self.files.lock().unwrap().insert(
                id.clone(),
                (
                    name.to_string(),
                    parent.to_string(),
                    mime.to_string(),
                    content,
                ),
            );
            id
        }
    }

    fn start_mock() -> (String, Arc<MockDrive>) {
        let drive = Arc::new(MockDrive {
            files: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
            auth_header: Mutex::new(None),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = drive.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let d = d.clone();
                std::thread::spawn(move || handle_conn(&mut stream, &d));
            }
        });
        (format!("http://127.0.0.1:{port}"), drive)
    }

    fn read_request(stream: &mut TcpStream) -> (String, String, Option<String>, Vec<u8>) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut content_length = 0usize;
        let mut auth = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            let lower = trimmed.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            } else if lower.starts_with("authorization:") {
                auth = Some(trimmed["authorization:".len()..].trim().to_string());
            }
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        let mut parts = request_line.split_whitespace();
        (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("").to_string(),
            auth,
            body,
        )
    }

    fn respond(stream: &mut TcpStream, status: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    }

    fn query_param(target: &str, key: &str) -> Option<String> {
        let q = target.split('?').nth(1)?;
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == key {
                    return Some(v.replace("+", " "));
                }
            }
        }
        None
    }

    fn handle_conn(stream: &mut TcpStream, drive: &MockDrive) {
        let (method, target, auth, body) = read_request(stream);
        if let Some(a) = auth {
            *drive.auth_header.lock().unwrap() = Some(a);
        }

        // PATCH /upload/drive/v3/files/<id>?uploadType=media — перезапись.
        if let Some(rest) = target.strip_prefix("/upload/drive/v3/files/") {
            let id = rest.split('?').next().unwrap_or("");
            let mut files = drive.files.lock().unwrap();
            if let Some(f) = files.get_mut(id) {
                f.3 = body;
                respond(stream, "200 OK", br#"{"id":"x"}"#);
            } else {
                respond(stream, "404 Not Found", b"{}");
            }
            return;
        }

        if target.starts_with("/drive/v3/files") && method == "GET" {
            // GET /files/<id>?alt=media или GET /files?q=...
            let path_part = target.trim_start_matches("/drive/v3/files");
            let path_part = path_part.split('?').next().unwrap_or("");
            if let Some(id) = path_part.strip_prefix('/') {
                if !id.is_empty() {
                    match drive.files.lock().unwrap().get(id) {
                        Some(f) => respond(stream, "200 OK", &f.3),
                        None => respond(stream, "404 Not Found", b"{}"),
                    }
                    return;
                }
            }
            // Список детей: q="'<parent>' in parents ..." (parent не экранирован
            // ureq — одинарные кавычки идут как %27).
            let q = query_param(&target, "q").unwrap_or_default();
            let q = q.replace("%27", "'");
            let parent = q
                .strip_prefix('\'')
                .and_then(|s| s.split("' in parents").next())
                .unwrap_or("")
                .to_string();
            let files = drive.files.lock().unwrap();
            let items: Vec<serde_json::Value> = files
                .iter()
                .filter(|(_, (_, p, _, _))| *p == parent)
                .map(|(id, (name, _, mime, _))| {
                    serde_json::json!({"id": id, "name": name, "mimeType": mime})
                })
                .collect();
            let body = serde_json::json!({"files": items});
            respond(stream, "200 OK", &serde_json::to_vec(&body).unwrap());
        } else if target.starts_with("/drive/v3/files") && method == "POST" {
            let meta: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let name = meta.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let parent = meta
                .get("parents")
                .and_then(|p| p.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mime = meta.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
            let id = drive.insert(name, parent, mime, Vec::new());
            let body = serde_json::json!({"id": id});
            respond(stream, "200 OK", &serde_json::to_vec(&body).unwrap());
        } else {
            respond(stream, "404 Not Found", b"{}");
        }
    }

    #[test]
    fn full_roundtrip_against_mock() {
        let (base, drive) = start_mock();
        let gd = GoogleDrive::with_base_url("test-token", &base);

        assert!(gd.get_index().unwrap().is_none());
        gd.put_index(br#"{"version":1}"#).unwrap();
        assert_eq!(gd.get_index().unwrap().unwrap(), br#"{"version":1}"#);
        // Перезапись индекса — тот же файл, не дубликат.
        gd.put_index(br#"{"version":2}"#).unwrap();
        assert_eq!(gd.get_index().unwrap().unwrap(), br#"{"version":2}"#);

        gd.put_snapshot("elden-ring", "v2_d_bbbbbbbb", b"zip2")
            .unwrap();
        gd.put_snapshot("elden-ring", "v1_d_aaaaaaaa", b"zip1")
            .unwrap();
        assert_eq!(
            gd.list_snapshots("elden-ring").unwrap(),
            vec!["v1_d_aaaaaaaa", "v2_d_bbbbbbbb"]
        );
        assert_eq!(
            gd.get_snapshot("elden-ring", "v1_d_aaaaaaaa").unwrap(),
            b"zip1"
        );
        assert!(gd.list_snapshots("absent").unwrap().is_empty());

        let err = gd.get_snapshot("elden-ring", "v9_nope").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        assert_eq!(
            drive.auth_header.lock().unwrap().as_deref(),
            Some("Bearer test-token")
        );
    }

    #[test]
    fn engine_works_over_google_backend() {
        let (base, _drive) = start_mock();
        let gd = GoogleDrive::with_base_url("token", &base);

        let dir = std::env::temp_dir().join(format!("savesync-test-gdrive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_dir = dir.join("home/game/saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        std::fs::write(save_dir.join("slot1.sav"), "progress").unwrap();

        let loc = crate::scanner::SaveLocation {
            pattern: "<home>/game/saves".into(),
            resolved: save_dir.to_string_lossy().replace('\\', "/"),
            hits: vec![save_dir.clone()],
        };
        let engine = crate::engine::Engine::new(gd, "pc", dir.join("state"));
        let outcome = engine.push("test-game", &[loc]).unwrap();
        assert!(matches!(outcome, crate::engine::PushOutcome::Uploaded(_)));

        let gd2 = GoogleDrive::with_base_url("token", &base);
        assert!(crate::engine::Engine::new(gd2, "deck", dir.join("state2"))
            .has_remote_versions("test-game")
            .unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }
}
