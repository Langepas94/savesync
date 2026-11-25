//! IPC-транспорт демона: JSON-RPC (core::ipc) поверх локального сокета —
//! named pipe на Windows, abstract unix socket на Linux/SteamOS
//! (namespaced-имена interprocess: кроссплатформенно, без файлового
//! cleanup). Одно соединение = поток; запросы newline-delimited.

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex, RwLock, TryLockError};

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Name};
use savesync_core::ipc::{Method, Request, Response};

use crate::runtime::Runtime;

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

    let cache = Arc::new(RwLock::new(
        runtime
            .lock()
            .map_err(|_| std::io::Error::other("daemon state poisoned"))?
            .status(),
    ));
    eprintln!("savesync-daemon: listening on {name}");
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let rt = Arc::clone(&runtime);
                let cache = cache.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, rt, cache);
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_conn(
    stream: LocalSocketStream,
    runtime: Arc<Mutex<Runtime>>,
    cache: Arc<RwLock<savesync_core::ipc::ResponsePayload>>,
) -> std::io::Result<()> {
    savesync_core::ipc::authorize_peer(&stream)?;
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
            Ok(req) => dispatch(req, &runtime, &cache),
            Err(e) => Response::err(0, format!("bad request: {e}")),
        };
        if write_response(&mut writer, response)? {
            // Shutdown was serialized by Runtime's mutex. Later requests are rejected.
            std::process::exit(0);
        }
    }
}

fn write_response(writer: &mut impl Write, response: Response) -> std::io::Result<bool> {
    let stopping = matches!(
        response.result,
        Some(savesync_core::ipc::ResponsePayload::Stopped)
    );
    let delivery = response
        .encode()
        .map_err(std::io::Error::other)
        .and_then(|encoded| writer.write_all(encoded.as_bytes()))
        .and_then(|()| writer.flush());
    // A disconnected stop caller must not leave a live, permanently stopping daemon.
    if stopping {
        return Ok(true);
    }
    delivery.map(|()| false)
}

fn dispatch(
    req: Request,
    runtime: &Arc<Mutex<Runtime>>,
    cache: &RwLock<savesync_core::ipc::ResponsePayload>,
) -> Response {
    if matches!(req.method, Method::Status) {
        return match runtime.try_lock() {
            Ok(runtime) => {
                if runtime.shutdown_requested {
                    return Response::err(req.id, "daemon is stopping");
                }
                match cache.read() {
                    Ok(value) => Response::ok(req.id, value.clone()),
                    Err(_) => Response::err(req.id, "status cache unavailable"),
                }
            }
            Err(TryLockError::WouldBlock) => match cache.read() {
                Ok(value) => {
                    let mut status = value.clone();
                    if let savesync_core::ipc::ResponsePayload::Status(status) = &mut status {
                        status.busy = true;
                    }
                    Response::ok(req.id, status)
                }
                Err(_) => Response::err(req.id, "status cache unavailable"),
            },
            Err(TryLockError::Poisoned(_)) => Response::err(req.id, "daemon state poisoned"),
        };
    }
    let mut rt = match runtime.lock() {
        Ok(rt) => rt,
        Err(_) => return Response::err(req.id, "daemon state poisoned"),
    };
    if rt.shutdown_requested {
        return Response::err(req.id, "daemon is stopping");
    }
    let payload = match &req.method {
        Method::Shutdown => rt.prepare_shutdown(),
        Method::Status => rt.status(),
        Method::Scan => rt.scan(),
        Method::ReloadManifests => rt.reload_manifests(),
        Method::SetGameHidden { game_key, hidden } => rt.set_game_hidden(game_key, *hidden),
        Method::Lan { action } => rt.lan_action(action.clone()),
        Method::OAuthBegin { provider } => rt.oauth_begin(*provider),
        Method::OAuthPoll { provider } => rt.oauth_poll(*provider),
        Method::OAuthCancel { provider } => rt.oauth_cancel(*provider),
        Method::Sync { game_key } => rt.sync(game_key.as_deref()),
        Method::History { game_key } => rt.history(game_key),
        Method::Restore {
            game_key,
            version_id,
        } => rt.restore(game_key, version_id),
        Method::Backup { game_key } => rt.backup_game(game_key),
        Method::GetConfig => rt.get_config(),
        Method::SaveConfig { config } => rt.save_config(*config.clone()),
    };
    if !rt.shutdown_requested {
        let status = rt.status();
        if let Ok(mut value) = cache.write() {
            *value = status;
        }
    }
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
    savesync_core::ipc::client::call(name, request)
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
            shutdown_requested: false,
            config,
            manifests: Default::default(),
            platform: savesync_core::paths::Platform::current(),
            roots: Default::default(),
            state_dir: dir.join("state"),
            last_scan: Vec::new(),
            hidden_games: Default::default(),
            lan_server: None,
            serving: false,
            oauth_jobs: Default::default(),
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
    #[test]
    fn shutdown_finishes_even_if_caller_disconnects() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        assert!(write_response(
            &mut Broken,
            Response::ok(1, savesync_core::ipc::ResponsePayload::Stopped)
        )
        .unwrap());
        assert!(write_response(&mut Broken, Response::err(1, "ordinary response")).is_err());
    }
    #[test]
    fn status_stays_responsive_while_runtime_is_busy() {
        let directory = std::env::temp_dir().join(format!("ss-busy-status-{}", std::process::id()));
        let runtime = Arc::new(Mutex::new(test_runtime(&directory)));
        let cache = Arc::new(RwLock::new(runtime.lock().unwrap().status()));
        let guard = runtime.lock().unwrap();
        let other = runtime.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            let _ = sender.send(dispatch(Request::new(1, Method::Status), &other, &cache));
        });
        let response = receiver.recv_timeout(std::time::Duration::from_secs(1));
        drop(guard);
        task.join().unwrap();
        assert!(
            matches!(response.unwrap().result,Some(ResponsePayload::Status(status))if status.busy)
        );
    }
}
