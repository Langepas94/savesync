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
    } else if req.target.starts_with("/v1/disk/resources") && req.method == "DELETE" {
        assert_eq!(
            query_param(&req.target, "permanently").as_deref(),
            Some("true")
        );
        assert!(path_param
            .rsplit('/')
            .next()
            .unwrap()
            .starts_with("savesync-access-check-"));
        disk.files.lock().unwrap().remove(&path_param).unwrap();
        respond(stream, "204 No Content", b"");
    } else if req.target.starts_with("/v1/disk/resources") && req.method == "GET" {
        let files = disk.files.lock().unwrap();
        let is_dir = disk.dirs.lock().unwrap().contains(&path_param);
        if is_dir || files.contains_key(&path_param) {
            let mut items: Vec<serde_json::Value> = files
                .keys()
                .filter_map(|k| k.strip_prefix(&format!("{path_param}/")))
                .filter(|rest| !rest.contains('/'))
                .map(|name| serde_json::json!({"name": name, "type": "file"}))
                .collect();
            items.sort_by_key(|value| value["name"].as_str().unwrap().to_string());
            let offset = query_param(&req.target, "offset")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let page = items
                .into_iter()
                .skip(offset)
                .take(1000)
                .collect::<Vec<_>>();
            let body = serde_json::json!({"_embedded": {"items": page}});
            respond(stream, "200 OK", &serde_json::to_vec(&body).unwrap());
        } else {
            respond(stream, "404 Not Found", b"{}");
        }
    } else {
        respond(stream, "404 Not Found", b"{}");
    }
}

#[test]
fn access_check_round_trips_in_app_folder_and_cleans_up() {
    let (base, disk) = start_mock();
    let mut backend = YandexDisk::with_base_url("test-token", base);
    backend.set_root("app:/SaveSync");
    disk.files
        .lock()
        .unwrap()
        .insert("app:/SaveSync/existing".into(), b"keep".to_vec());
    backend.verify_access().unwrap();
    let files = disk.files.lock().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files["app:/SaveSync/existing"], b"keep");
}

#[test]
fn full_roundtrip_against_mock() {
    let (base, disk) = start_mock();
    let mut ya = YandexDisk::with_base_url("test-token", &base);
    ya.set_root("disk:/SaveSync");

    // Индекс: None → запись → чтение.
    assert!(ya.get_index().unwrap().is_none());
    ya.put_index(br#"{"version":1}"#).unwrap();
    assert_eq!(
        crate::index_journal::parse(&ya.get_index().unwrap().unwrap()).unwrap(),
        crate::index::Index::new()
    );

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
#[test]
fn stale_concurrent_writers_keep_every_version() {
    let (base, _) = start_mock();
    let seed = crate::index_journal::tests::branch("seed", None);
    YandexDisk::with_base_url("token", &base)
        .publish_index(&serde_json::to_vec(&seed).unwrap(), "seed")
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for number in 0..8 {
        let base = base.clone();
        let barrier = barrier.clone();
        tasks.push(std::thread::spawn(move || {
            let backend = YandexDisk::with_base_url("token", base);
            let mut index =
                crate::index_journal::parse(&backend.get_index().unwrap().unwrap()).unwrap();
            let writer = format!("writer{number}");
            index.merge(&crate::index_journal::tests::branch(
                &writer,
                Some("v1_seed_aaaaaaaa"),
            ));
            barrier.wait();
            backend
                .publish_index(&serde_json::to_vec(&index).unwrap(), &writer)
                .unwrap();
        }));
    }
    for task in tasks {
        task.join().unwrap();
    }
    let index = crate::index_journal::parse(
        &YandexDisk::with_base_url("token", base)
            .get_index()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(index.game("game").unwrap().versions.len(), 9);
    assert_eq!(index.game("game").unwrap().heads().len(), 8);
}
#[test]
fn listing_reads_every_page() {
    let (base, disk) = start_mock();
    disk.dirs.lock().unwrap().insert("disk:/SaveSync".into());
    for number in 0..1005 {
        disk.files
            .lock()
            .unwrap()
            .insert(format!("disk:/SaveSync/file{number}"), vec![]);
    }
    let backend = YandexDisk::with_base_url("token", base);
    assert_eq!(backend.list_files("").unwrap().len(), 1005);
}

#[test]
fn unchanged_publications_do_not_grow_remote_history() {
    let (base, disk) = start_mock();
    let backend = YandexDisk::with_base_url("token", base);
    let index = crate::index_journal::tests::branch("pc", None);
    let bytes = serde_json::to_vec(&index).unwrap();
    backend.publish_index(&bytes, "pc").unwrap();
    let count = disk.files.lock().unwrap().len();
    for _ in 0..5 {
        backend.get_index().unwrap();
        backend.publish_index(&bytes, "pc").unwrap();
    }
    assert_eq!(disk.files.lock().unwrap().len(), count);
}
