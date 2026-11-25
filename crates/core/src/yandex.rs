//! Бэкенд Яндекс.Диска (plan §2.1): REST API `cloud-api.yandex.net`.
//!
//! Аутентификация — OAuth-токен в конфиге (`backends.yandex.token`).
//! Корень данных: `disk:/SaveSync`.
//!
//! `put_index` перезаливает индекс целиком после merge: снапшоты
//! иммутабельны, проигравшая гонка восстанавливается повторным merge
//! при следующем sync.

use std::io::{self, Read};
use std::time::Duration;

const DEFAULT_BASE: &str = "https://cloud-api.yandex.net/v1/disk";

/// Канал синхронизации через Яндекс.Диск.
pub struct YandexDisk {
    token: String,
    /// Корень на диске, например `disk:/SaveSync`.
    root: String,
    /// База REST API (в тестах подменяется mock-сервером).
    base_url: String,
    agent: ureq::Agent,
}

impl YandexDisk {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(token, DEFAULT_BASE)
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build();
        YandexDisk {
            token: token.into(),
            root: "disk:/SaveSync".into(),
            base_url: base_url.into(),
            agent,
        }
    }

    /// Корень данных на диске (для тестов/отладки).
    pub fn set_root(&mut self, root: impl Into<String>) {
        self.root = root.into();
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        req.set("Authorization", &format!("OAuth {}", self.token))
    }

    /// Полный путь ресурса на диске: `<root>/<rel>`.
    fn disk_path(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{rel}", self.root)
        }
    }

    fn api_error(&self, op: &str, err: ureq::Error) -> io::Error {
        let kind = match &err {
            ureq::Error::Status(401 | 403, _) => io::ErrorKind::PermissionDenied,
            ureq::Error::Status(404, _) => io::ErrorKind::NotFound,
            ureq::Error::Status(_, _) => io::ErrorKind::Other,
            ureq::Error::Transport(_) => io::ErrorKind::ConnectionAborted,
        };
        io::Error::new(kind, format!("yandex disk {op}: {err}"))
    }

    /// GET JSON с query-параметрами.
    fn get_json(
        &self,
        op: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> io::Result<serde_json::Value> {
        let mut req = self
            .auth(self.agent.get(&format!("{}{path}", self.base_url)))
            .set("Accept", "application/json");
        for (k, v) in query {
            req = req.query(k, v);
        }
        match req.call() {
            Ok(resp) => resp
                .into_json()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) => Err(self.api_error(op, e)),
        }
    }

    /// Создать директорию (409 «уже есть» — не ошибка).
    fn mkdir(&self, rel: &str) -> io::Result<()> {
        let req = self
            .auth(self.agent.put(&format!("{}/resources", self.base_url)))
            .query("path", &self.disk_path(rel));
        match req.call() {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(409, _)) => Ok(()),
            Err(e) => Err(self.api_error("mkdir", e)),
        }
    }

    /// Загрузка байтов по пути (перезапись).
    fn upload(&self, rel: &str, data: &[u8]) -> io::Result<()> {
        let link = self.get_json(
            "upload-href",
            "/resources/upload",
            &[("path", &self.disk_path(rel)), ("overwrite", "true")],
        )?;
        let href = link
            .get("href")
            .and_then(|h| h.as_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no href in upload link"))?;
        // href указывает на uploader.yandex.net; в тестах — на mock.
        self.agent
            .put(href)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(data)
            .map_err(|e| self.api_error("upload", e))?;
        Ok(())
    }

    /// Скачивание байтов по пути.
    fn download(&self, rel: &str) -> io::Result<Vec<u8>> {
        let link = self.get_json(
            "download-href",
            "/resources/download",
            &[("path", &self.disk_path(rel))],
        )?;
        let href = link.get("href").and_then(|h| h.as_str()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "no href in download link")
        })?;
        let resp = self
            .agent
            .get(href)
            .call()
            .map_err(|e| self.api_error("download", e))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(512 * 1024 * 1024)
            .read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Имена файлов в директории (директории пропускаются).
    fn list_files(&self, rel: &str) -> io::Result<Vec<String>> {
        let resp = match self.get_json(
            "list",
            "/resources",
            &[
                ("path", &self.disk_path(rel)),
                ("limit", "1000"),
                ("fields", "_embedded.items.name,_embedded.items.type"),
            ],
        ) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        if let Some(items) = resp
            .get("_embedded")
            .and_then(|e| e.get("items"))
            .and_then(|i| i.as_array())
        {
            for item in items {
                if item.get("type").and_then(|t| t.as_str()) == Some("file") {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        out.push(name.to_string());
                    }
                }
            }
        }
        Ok(out)
    }

    fn ensure_dirs(&self) -> io::Result<()> {
        self.mkdir("")?;
        self.mkdir("saves")
    }
}

impl crate::backend::Backend for YandexDisk {
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()> {
        self.ensure_dirs()?;
        self.mkdir(&format!("saves/{game_key}"))?;
        self.upload(&format!("saves/{game_key}/{version_id}.zip"), data)
    }

    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>> {
        self.download(&format!("saves/{game_key}/{version_id}.zip"))
    }

    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>> {
        let mut out: Vec<String> = self
            .list_files(&format!("saves/{game_key}"))?
            .into_iter()
            .filter_map(|n| n.strip_suffix(".zip").map(str::to_string))
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
        self.ensure_dirs()?;
        self.upload("index.json", data)
    }

    fn name(&self) -> &'static str {
        "yandex-disk"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use std::collections::{HashMap, HashSet};
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    /// Мини-HTTP-сервер, эмулирующий подмножество Яндекс REST API:
    /// файлы и «директории» (созданные через PUT /resources) в памяти.
    struct MockDisk {
        files: Mutex<HashMap<String, Vec<u8>>>,
        dirs: Mutex<HashSet<String>>,
        auth_header: Mutex<Option<String>>,
    }

    fn start_mock() -> (String, Arc<MockDisk>) {
        let disk = Arc::new(MockDisk {
            files: Mutex::new(HashMap::new()),
            dirs: Mutex::new(HashSet::new()),
            auth_header: Mutex::new(None),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let d = disk.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let d = d.clone();
                std::thread::spawn(move || handle_conn(&mut stream, &d));
            }
        });
        (format!("http://127.0.0.1:{port}/v1/disk"), disk)
    }

    struct HttpRequest {
        method: String,
        target: String,
        auth: Option<String>,
        host: String,
        body: Vec<u8>,
    }

    fn read_request(stream: &mut TcpStream) -> HttpRequest {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut content_length = 0usize;
        let mut auth = None;
        let mut host = String::new();
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
            } else if let Some(v) = lower.strip_prefix("host:") {
                host = v.trim().to_string();
            }
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        let mut parts = request_line.split_whitespace();
        HttpRequest {
            method: parts.next().unwrap_or("").to_string(),
            target: parts.next().unwrap_or("").to_string(),
            auth,
            host,
            body,
        }
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
                    return Some(url_decode(v));
                }
            }
        }
        None
    }

    fn url_decode(s: &str) -> String {
        let mut out = Vec::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' if i + 2 < bytes.len() => {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                    out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
                    i += 3;
                }
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn handle_conn(stream: &mut TcpStream, disk: &MockDisk) {
        let req = read_request(stream);
        if let Some(auth) = &req.auth {
            *disk.auth_header.lock().unwrap() = Some(auth.clone());
        }

        // href загрузки/скачивания указывают на тот же mock.
        if let Some(rest) = req.target.strip_prefix("/upload/") {
            let path = url_decode(rest);
            disk.files.lock().unwrap().insert(path, req.body);
            respond(stream, "201 Created", b"");
            return;
        }
        if let Some(rest) = req.target.strip_prefix("/download/") {
            let path = url_decode(rest);
            match disk.files.lock().unwrap().get(&path) {
                Some(data) => respond(stream, "200 OK", data),
                None => respond(stream, "404 Not Found", b"{}"),
            }
            return;
        }

        let path_param = query_param(&req.target, "path").unwrap_or_default();
        let base = format!("http://{}", req.host);
        if req.target.starts_with("/v1/disk/resources/upload") {
            let body =
                serde_json::json!({"href": format!("{base}/upload/{path_param}"), "method": "PUT"});
            respond(stream, "200 OK", &serde_json::to_vec(&body).unwrap());
        } else if req.target.starts_with("/v1/disk/resources/download") {
            if disk.files.lock().unwrap().contains_key(&path_param) {
                let body = serde_json::json!({"href": format!("{base}/download/{path_param}")});
                respond(stream, "200 OK", &serde_json::to_vec(&body).unwrap());
            } else {
                respond(stream, "404 Not Found", b"{}");
            }
        } else if req.target.starts_with("/v1/disk/resources") && req.method == "PUT" {
            disk.dirs.lock().unwrap().insert(path_param);
            respond(stream, "201 Created", b"{}");
        } else if req.target.starts_with("/v1/disk/resources") && req.method == "GET" {
            let files = disk.files.lock().unwrap();
            let is_dir = disk.dirs.lock().unwrap().contains(&path_param);
            if is_dir || files.contains_key(&path_param) {
                let items: Vec<serde_json::Value> = files
                    .keys()
                    .filter_map(|k| k.strip_prefix(&format!("{path_param}/")))
                    .filter(|rest| !rest.contains('/'))
                    .map(|name| serde_json::json!({"name": name, "type": "file"}))
                    .collect();
                let body = serde_json::json!({"_embedded": {"items": items}});
                respond(stream, "200 OK", &serde_json::to_vec(&body).unwrap());
            } else {
                respond(stream, "404 Not Found", b"{}");
            }
        } else {
            respond(stream, "404 Not Found", b"{}");
        }
    }

    #[test]
    fn full_roundtrip_against_mock() {
        let (base, disk) = start_mock();
        let mut ya = YandexDisk::with_base_url("test-token", &base);
        ya.set_root("disk:/SaveSync");

        // Индекс: None → запись → чтение.
        assert!(ya.get_index().unwrap().is_none());
        ya.put_index(br#"{"version":1}"#).unwrap();
        assert_eq!(ya.get_index().unwrap().unwrap(), br#"{"version":1}"#);

        // Снапшоты: запись, листинг, чтение.
        ya.put_snapshot("elden-ring", "v2_d_bbbbbbbb", b"zip2")
            .unwrap();
        ya.put_snapshot("elden-ring", "v1_d_aaaaaaaa", b"zip1")
            .unwrap();
        assert_eq!(
            ya.list_snapshots("elden-ring").unwrap(),
            vec!["v1_d_aaaaaaaa", "v2_d_bbbbbbbb"]
        );
        assert_eq!(
            ya.get_snapshot("elden-ring", "v1_d_aaaaaaaa").unwrap(),
            b"zip1"
        );
        assert!(ya.list_snapshots("absent").unwrap().is_empty());

        // Несуществующий снапшот — NotFound.
        let err = ya.get_snapshot("elden-ring", "v9_nope").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        // Токен уходит в Authorization.
        assert_eq!(
            disk.auth_header.lock().unwrap().as_deref(),
            Some("OAuth test-token")
        );
    }

    #[test]
    fn engine_works_over_yandex_backend() {
        let (base, _disk) = start_mock();
        let mut ya = YandexDisk::with_base_url("token", &base);
        ya.set_root("disk:/SaveSync");

        // Полный цикл engine поверх mock-облака: push → pull на «другом»
        // устройстве. Сейв — обычный файл в temp.
        let dir = std::env::temp_dir().join(format!("savesync-test-yandex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_dir = dir.join("home/game/saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        std::fs::write(save_dir.join("slot1.sav"), "progress").unwrap();

        let loc = crate::scanner::SaveLocation {
            location: None,
            pattern: "<home>/game/saves".into(),
            resolved: save_dir.to_string_lossy().replace('\\', "/"),
            hits: vec![save_dir.clone()],
        };
        let engine = crate::engine::Engine::new(ya, "pc", dir.join("state"));
        let outcome = engine.push("test-game", &[loc]).unwrap();
        assert!(matches!(outcome, crate::engine::PushOutcome::Uploaded(_)));

        // Второй «девайс» поверх того же облака.
        let ya2 = YandexDisk::with_base_url("token", &base);
        assert!(crate::engine::Engine::new(ya2, "deck", dir.join("state2"))
            .has_remote_versions("test-game")
            .unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }
}
