//! savesync-daemon — фоновый демон синхронизации сейвов.
//!
//! Команды:
//!   serve              запустить демон (IPC-сервер, unix socket / named pipe)
//!   status             статус (через демон, если запущен)
//!   scan               пересканировать игры
//!   sync [game_key]    ручная синхронизация (всех или одной игры)
//!
//! scan/sync/status сначала пробуют IPC к запущенному демону; если демон
//! не запущен — scan/sync выполняются напрямую (offline-режим CLI).

mod ipc;
mod runtime;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use savesync_core::config::{data_dir, Config};
use savesync_core::ipc::{Method, Request, ResponsePayload};
use savesync_core::paths::Platform;

use runtime::Runtime;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("serve") => cmd_serve(),
        Some("scan") => cmd_via_daemon_or_direct(Method::Scan),
        Some("status") => cmd_via_daemon(Method::Status),
        Some("sync") => {
            let game = args.get(1).cloned();
            cmd_via_daemon_or_direct(Method::Sync { game_key: game })
        }
        _ => {
            eprintln!(
                "savesync-daemon {}\nкоманды: serve | status | scan | sync [game_key]",
                env!("CARGO_PKG_VERSION")
            );
            2
        }
    };
    std::process::exit(code);
}

fn config_path() -> PathBuf {
    data_dir(Platform::current())
        .unwrap_or_else(|| PathBuf::from(".savesync"))
        .join("config.json")
}

fn load_runtime() -> Result<Runtime, String> {
    let cfg_path = config_path();
    let config = Config::load_or_init(&cfg_path).map_err(|e| format!("config: {e}"))?;
    let state = cfg_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("state");
    Runtime::new(config, state).map_err(|e| format!("init: {e}"))
}

fn cmd_serve() -> i32 {
    let runtime = match load_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return 1;
        }
    };
    match ipc::serve(Arc::new(Mutex::new(runtime)), ipc::SOCKET_NAME) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ошибка IPC-сервера: {e}");
            1
        }
    }
}

/// Команда только через демон (status без демона бессмысленен).
fn cmd_via_daemon(method: Method) -> i32 {
    match ipc::call(ipc::SOCKET_NAME, Request::new(1, method)) {
        Ok(resp) => print_response(resp),
        Err(_) => {
            eprintln!("демон не запущен (savesync-daemon serve)");
            1
        }
    }
}

/// Команда через демон, либо напрямую при его отсутствии.
fn cmd_via_daemon_or_direct(method: Method) -> i32 {
    if let Ok(resp) = ipc::call(ipc::SOCKET_NAME, Request::new(1, method.clone())) {
        return print_response(resp);
    }
    let mut rt = match load_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return 1;
        }
    };
    let payload = match &method {
        Method::Scan => rt.scan(),
        Method::Sync { game_key } => rt.sync(game_key.as_deref()),
        Method::Status => rt.status(),
    };
    print_payload(&payload);
    0
}

fn print_response(resp: savesync_core::ipc::Response) -> i32 {
    match (resp.result, resp.error) {
        (Some(payload), _) => {
            print_payload(&payload);
            0
        }
        (None, Some(err)) => {
            eprintln!("ошибка демона: {}", err.message);
            1
        }
        (None, None) => 0,
    }
}

fn print_payload(payload: &ResponsePayload) {
    match serde_json::to_string_pretty(payload) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("serialization error: {e}"),
    }
}
