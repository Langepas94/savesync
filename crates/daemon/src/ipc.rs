//! IPC-транспорт демона: JSON-RPC (core::ipc) поверх локального сокета —
//! named pipe на Windows, abstract unix socket на Linux/SteamOS
//! (namespaced-имена interprocess: кроссплатформенно, без файлового
//! cleanup). Одно соединение = поток; запросы newline-delimited.

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Name};
use savesync_core::ipc::{Method, Request, Response};

use crate::runtime::Runtime;

/// Имя сокета демона по умолчанию.
pub const SOCKET_NAME: &str = "savesync-daemon";

fn ns_name(name: &str) -> std::io::Result<Name<'static>> {
    name.to_owned()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
}

/// Запускает блокирующий IPC-сервер. Возвращает AddrInUse, если демон
/// уже запущен (второй инстанс не поднимается).
pub fn serve(runtime: Arc<Mutex<Runtime>>, name: &str) -> std::io::Result<()> {
    let listener: LocalSocketListener =
        ListenerOptions::new().name(ns_name(name)?).create_sync()?;

    eprintln!("savesync-daemon: listening on {name}");
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let rt = Arc::clone(&runtime);
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, rt);
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(stream: LocalSocketStream, runtime: Arc<Mutex<Runtime>>) -> std::io::Result<()> {
    // &Stream реализует и Read, и Write — клонирование не требуется.
    let mut writer = &stream;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(()); // клиент отключился
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = match Request::decode(&line) {
            Ok(req) => dispatch(req, &runtime),
            Err(e) => Response::err(0, format!("bad request: {e}")),
        };
        writer.write_all(response.encode()?.as_bytes())?;
        writer.flush()?;
    }
}

fn dispatch(req: Request, runtime: &Arc<Mutex<Runtime>>) -> Response {
    let mut rt = match runtime.lock() {
        Ok(rt) => rt,
        Err(_) => return Response::err(req.id, "daemon state poisoned"),
    };
    let payload = match &req.method {
        Method::Status => rt.status(),
        Method::Scan => rt.scan(),
        Method::Sync { game_key } => rt.sync(game_key.as_deref()),
    };
    Response::ok(req.id, payload)
}

/// Есть ли живой демон на сокете.
// Используется в тестах и пригодится CLI для health-check; в релизной
// сборке bin-крейта пока не вызывается.
#[cfg_attr(not(test), allow(dead_code))]
pub fn probe(name: &str) -> bool {
    call(name, Request::new(0, Method::Status)).is_ok()
}

/// Клиентский вызов: один запрос — одно соединение — один ответ.
pub fn call(name: &str, request: Request) -> std::io::Result<Response> {
    let mut stream = LocalSocketStream::connect(ns_name(name)?)?;
    stream.write_all(
        request
            .encode()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            .as_bytes(),
    )?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "daemon closed connection",
        ));
    }
    Response::decode(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use savesync_core::ipc::ResponsePayload;

    fn test_runtime(dir: &std::path::Path) -> Runtime {
        let config = savesync_core::config::Config {
            version: 1,
            device_id: "test-device".into(),
            device_name: "Test Device".into(),
            manifests_dir: None,
            backends: Default::default(),
            backups: Default::default(),
        };
        Runtime {
            config,
            manifests: Default::default(),
            platform: savesync_core::paths::Platform::current(),
            roots: Default::default(),
            state_dir: dir.join("state"),
            last_scan: Vec::new(),
        }
    }

    fn unique_name(test: &str) -> String {
        format!("savesync-test-{test}-{}", std::process::id())
    }

    fn wait_up(name: &str) {
        for _ in 0..100 {
            if probe(name) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("daemon did not start on {name}");
    }

    #[test]
    fn ipc_roundtrip_status_and_scan() {
        let dir = std::env::temp_dir().join(format!("savesync-test-ipc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = unique_name("roundtrip");

        let runtime = Arc::new(Mutex::new(test_runtime(&dir)));
        let server_name = name.clone();
        std::thread::spawn(move || serve(runtime, &server_name));
        wait_up(&name);

        let resp = call(&name, Request::new(1, Method::Status)).unwrap();
        match resp.result {
            Some(ResponsePayload::Status(s)) => {
                assert_eq!(s.device_id, "test-device");
                assert_eq!(s.device_name, "Test Device");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let resp = call(&name, Request::new(2, Method::Scan)).unwrap();
        match resp.result {
            Some(ResponsePayload::Scan(s)) => assert_eq!(s.found, 0),
            other => panic!("unexpected: {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn second_daemon_instance_refused() {
        let dir = std::env::temp_dir().join(format!("savesync-test-ipc2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = unique_name("second");

        let rt1 = Arc::new(Mutex::new(test_runtime(&dir)));
        let n1 = name.clone();
        std::thread::spawn(move || serve(rt1, &n1));
        wait_up(&name);

        let rt2 = Arc::new(Mutex::new(test_runtime(&dir)));
        let err = serve(rt2, &name).unwrap_err();
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::AddrInUse
                    | std::io::ErrorKind::AlreadyExists
                    | std::io::ErrorKind::PermissionDenied
            ),
            "unexpected: {err}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
