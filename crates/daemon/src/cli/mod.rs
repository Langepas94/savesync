use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use savesync_core::config::{data_dir, Config};
use savesync_core::ipc::{Method, Request, ResponsePayload};
use savesync_core::platform::paths::Platform;

use crate::{ipc, service::Runtime};

mod backup;
mod custom;
mod remote;
mod service;
pub(crate) use backup::cmd_backup;
pub(crate) use custom::cmd_custom;
pub(crate) use remote::cmd_remote;
pub(crate) use service::cmd_serve;

mod manifests;
pub(crate) use manifests::cmd_manifests;

mod authorization;

mod networking;

pub(crate) fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("savesync-daemon {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("serve") => cmd_serve(),
        Some("stop") => {
            if !ipc::probe(&savesync_core::ipc::client::socket_name()) {
                0
            } else {
                cmd_via_daemon(Method::Shutdown)
            }
        }
        Some("scan") => cmd_via_daemon_or_direct(Method::Scan),
        Some("status") => cmd_via_daemon(Method::Status),
        Some("sync") => {
            let game = args.get(1).cloned();
            cmd_via_daemon_or_direct(Method::Sync { game_key: game })
        }
        Some("custom") => cmd_custom(&args[1..]),
        Some("manifests") => cmd_manifests(&args[1..]),
        Some("history") if args.len() == 2 => cmd_via_daemon_or_direct(Method::History {
            game_key: args[1].clone(),
        }),
        Some("restore") if args.len() == 3 => cmd_via_daemon_or_direct(Method::Restore {
            game_key: args[1].clone(),
            version_id: args[2].clone(),
        }),
        Some("remote") => cmd_remote(&args[1..]),
        Some("backup") => cmd_backup(&args[1..]),
        _ => {
            eprintln!(
                "savesync-daemon {}\nкоманды: serve | stop | status | scan | sync [game_key] | history <game_key> | restore <game_key> <version_id> | custom <add|list|remove> | remote <login|logout|setup|peer|list> | backup <create|list|import|dir>",
                env!("CARGO_PKG_VERSION")
            );
            2
        }
    }
}

fn config_path() -> PathBuf {
    data_dir(Platform::current())
        .unwrap_or_else(|| PathBuf::from(".savesync"))
        .join("config.json")
}

fn state_dir() -> PathBuf {
    config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("state")
}

/// Custom-правила пользователя: `<state>/custom/<slug>.yaml` (schema §3).
fn load_runtime() -> Result<Runtime, String> {
    let cfg_path = config_path();
    let config = Config::load_or_init(&cfg_path).map_err(|e| format!("config: {e}"))?;
    Runtime::new(config, state_dir()).map_err(|e| format!("init: {e}"))
}

/// Команда только через демон (status без демона бессмысленен).
fn cmd_via_daemon(method: Method) -> i32 {
    match ipc::call(
        &savesync_core::ipc::client::socket_name(),
        Request::new(1, method),
    ) {
        Ok(resp) => print_response(resp),
        Err(_) => {
            eprintln!("демон не запущен (savesync-daemon serve)");
            1
        }
    }
}

/// Команда через демон, либо напрямую при его отсутствии.
fn cmd_via_daemon_or_direct(method: Method) -> i32 {
    if let Ok(resp) = ipc::call(
        &savesync_core::ipc::client::socket_name(),
        Request::new(1, method.clone()),
    ) {
        return print_response(resp);
    }
    let mut rt = match load_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return 1;
        }
    };
    let result = match &method {
        Method::LanPresence => match crate::service::lan_presence(rt.lan_presence_directory()) {
            Ok(online) => Ok(ResponsePayload::LanPresence(online)),
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        },
        Method::CloudCheck { provider } => Ok(ResponsePayload::OAuthStatus(
            crate::service::cloud_check(*provider, rt.cloud_reference(*provider)),
        )),
        _ => ipc::dispatch::execute(&mut rt, &method),
    };
    let payload = match result {
        Ok(payload) => payload,
        Err(error) => {
            eprintln!("ошибка: {error}");
            return 1;
        }
    };
    print_payload(&payload);
    payload_exit_code(&payload)
}

fn print_response(resp: savesync_core::ipc::Response) -> i32 {
    match (resp.result, resp.error) {
        (Some(payload), _) => {
            print_payload(&payload);
            payload_exit_code(&payload)
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

fn payload_exit_code(payload: &ResponsePayload) -> i32 {
    match payload {
        ResponsePayload::Lan(status) if status.error.is_some() => 1,
        ResponsePayload::OAuthStatus(status) if status.state == "error" => 1,
        ResponsePayload::BackupBatch(batch) if !batch.errors.is_empty() => 1,
        ResponsePayload::Sync(sync)
            if sync.results.iter().any(|result| {
                matches!(
                    result.outcome.as_str(),
                    "error" | "locked" | "conflict" | "skipped"
                )
            }) =>
        {
            1
        }
        _ => 0,
    }
}
