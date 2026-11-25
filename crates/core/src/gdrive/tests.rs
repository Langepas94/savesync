use super::*;
use crate::backend::Backend;
use std::collections::HashMap;
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
    sessions: Mutex<HashMap<String, serde_json::Value>>,
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
        sessions: Mutex::new(HashMap::new()),
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

    if target.starts_with("/upload/drive/v3/files?") && method == "POST" {
        if query_param(&target, "uploadType").as_deref() == Some("resumable") {
            let mut count = drive.next_id.lock().unwrap();
            *count += 1;
            let session = format!("session{}", *count);
            drive
                .sessions
                .lock()
                .unwrap()
                .insert(session.clone(), serde_json::from_slice(&body).unwrap());
            let location = format!("http://{}/session/{session}", stream.local_addr().unwrap());
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nLocation: {location}\r\nConnection: close\r\n\r\n").unwrap();
            return;
        }
        let boundary_end = body.windows(2).position(|bytes| bytes == b"\r\n").unwrap();
        let boundary = &body[..boundary_end];
        let delimiter = [b"\r\n".as_slice(), boundary].concat();
        let meta_start = body
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        let meta_end = meta_start
            + body[meta_start..]
                .windows(delimiter.len())
                .position(|bytes| bytes == delimiter)
                .unwrap();
        let metadata: serde_json::Value =
            serde_json::from_slice(&body[meta_start..meta_end]).unwrap();
        let data_start = meta_end
            + delimiter.len()
            + body[meta_end + delimiter.len()..]
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
            + 4;
        let data_end = data_start
            + body[data_start..]
                .windows(delimiter.len())
                .position(|bytes| bytes == delimiter)
                .unwrap();
        let id = drive.insert(
            metadata["name"].as_str().unwrap(),
            metadata["parents"][0].as_str().unwrap(),
            "application/octet-stream",
            body[data_start..data_end].to_vec(),
        );
        respond(
            stream,
            "200 OK",
            &serde_json::to_vec(&serde_json::json!({"id":id})).unwrap(),
        );
        return;
    }
    if let Some(session) = target.strip_prefix("/session/") {
        let metadata = drive.sessions.lock().unwrap().remove(session).unwrap();
        let id = drive.insert(
            metadata["name"].as_str().unwrap(),
            metadata["parents"][0].as_str().unwrap(),
            "application/octet-stream",
            body,
        );
        respond(
            stream,
            "200 OK",
            &serde_json::to_vec(&serde_json::json!({"id":id})).unwrap(),
        );
        return;
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
        let mut items: Vec<serde_json::Value> = files
                .iter()
                .filter(|(_, (_, p, _, _))| *p == parent)
                .map(|(id, (name, _, mime, _))| {
                    serde_json::json!({"id": id, "name": name, "mimeType": mime})
                })
                .collect();
        items.sort_by_key(|value| value["id"].as_str().unwrap().to_string());
        let offset = query_param(&target, "pageToken")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let page = items
            .iter()
            .skip(offset)
            .take(1000)
            .cloned()
            .collect::<Vec<_>>();
        let mut body = serde_json::json!({"files": page});
        if offset + 1000 < items.len() {
            body["nextPageToken"] = (offset + 1000).to_string().into();
        }
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
    assert_eq!(
        index_journal::parse(&gd.get_index().unwrap().unwrap()).unwrap(),
        crate::index::Index::new()
    );
    // Repeated publication is idempotent in the merged view.
    gd.put_index(br#"{"version":1}"#).unwrap();
    assert_eq!(
        index_journal::parse(&gd.get_index().unwrap().unwrap()).unwrap(),
        crate::index::Index::new()
    );

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
        location: None,
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

#[test]
fn stale_concurrent_writers_keep_every_version() {
    let (base, _) = start_mock();
    let seed = crate::index_journal::tests::branch("seed", None);
    let bytes = serde_json::to_vec(&seed).unwrap();
    GoogleDrive::with_base_url("token", &base)
        .publish_index(&bytes, "seed")
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for number in 0..8 {
        let base = base.clone();
        let barrier = barrier.clone();
        tasks.push(std::thread::spawn(move || {
            let backend = GoogleDrive::with_base_url("token", base);
            let mut index = index_journal::parse(&backend.get_index().unwrap().unwrap()).unwrap();
            let writer = format!("writer{number}");
            let branch = crate::index_journal::tests::branch(&writer, Some("v1_seed_aaaaaaaa"));
            index.merge(&branch);
            barrier.wait();
            backend
                .publish_index(&serde_json::to_vec(&index).unwrap(), &writer)
                .unwrap();
        }));
    }
    for task in tasks {
        task.join().unwrap();
    }
    let index = index_journal::parse(
        &GoogleDrive::with_base_url("token", base)
            .get_index()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(index.game("game").unwrap().versions.len(), 9);
    assert_eq!(index.game("game").unwrap().heads().len(), 8);
}
#[test]
fn snapshots_over_five_megabytes_use_resumable_upload() {
    let (base, _) = start_mock();
    let backend = GoogleDrive::with_base_url("token", base);
    let bytes = vec![42; 5 * 1024 * 1024 + 1];
    backend
        .put_snapshot("game", "v1_pc_aaaaaaaa", &bytes)
        .unwrap();
    assert_eq!(
        backend.get_snapshot("game", "v1_pc_aaaaaaaa").unwrap(),
        bytes
    );
}
#[test]
fn legacy_duplicate_roots_are_all_read_and_not_overwritten() {
    let (base, drive) = start_mock();
    for writer in ["pc", "deck"] {
        let root = drive.insert("SaveSync", ROOT_SPACE, FOLDER_MIME, vec![]);
        drive.insert(
            "index.json",
            &root,
            "application/json",
            serde_json::to_vec(&crate::index_journal::tests::branch(writer, None)).unwrap(),
        );
    }
    let backend = GoogleDrive::with_base_url("token", base);
    let index = index_journal::parse(&backend.get_index().unwrap().unwrap()).unwrap();
    assert_eq!(index.game("game").unwrap().heads().len(), 2);
}

#[test]
fn listing_reads_every_page() {
    let (base, drive) = start_mock();
    for number in 0..1005 {
        drive.insert(
            &format!("file{number}"),
            ROOT_SPACE,
            "application/octet-stream",
            vec![],
        );
    }
    let backend = GoogleDrive::with_base_url("token", base);
    assert_eq!(backend.list_children(ROOT_SPACE).unwrap().len(), 1005);
}

#[test]
fn unchanged_publications_do_not_grow_remote_history() {
    let (base, drive) = start_mock();
    let backend = GoogleDrive::with_base_url("token", base);
    let index = crate::index_journal::tests::branch("pc", None);
    let bytes = serde_json::to_vec(&index).unwrap();
    backend.publish_index(&bytes, "pc").unwrap();
    let count = drive.files.lock().unwrap().len();
    for _ in 0..5 {
        backend.get_index().unwrap();
        backend.publish_index(&bytes, "pc").unwrap();
    }
    assert_eq!(drive.files.lock().unwrap().len(), count);
}
