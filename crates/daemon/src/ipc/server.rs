//! IPC-транспорт демона: JSON-RPC (core::ipc) поверх локального сокета —
//! named pipe на Windows, abstract unix socket на Linux/SteamOS
//! (namespaced-имена interprocess: кроссплатформенно, без файлового
//! cleanup). Одно соединение = поток; запросы newline-delimited.

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex, RwLock};

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Name};
use savesync_core::ipc::{Method, Request, Response, ResponsePayload, StatusPayload};

use super::dispatch::dispatch;

use crate::service::Runtime;

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
    cache: Arc<RwLock<StatusPayload>>,
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
    let stopping = matches!(response.result, Some(ResponsePayload::Stopped));
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
#[path = "../tests/server.rs"]
mod tests;
