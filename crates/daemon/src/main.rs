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
        Some("custom") => cmd_custom(&args[1..]),
        Some("remote") => cmd_remote(&args[1..]),
        _ => {
            eprintln!(
                "savesync-daemon {}\nкоманды: serve | status | scan | sync [game_key] | custom <add|list|remove> | remote <setup|list>",
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

fn state_dir() -> PathBuf {
    config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("state")
}

/// Custom-правила пользователя: `<state>/custom/<slug>.yaml` (schema §3).
fn custom_dir() -> PathBuf {
    state_dir().join("custom")
}

/// custom add <name> --path <p> [--path ...] [--os any|windows|linux]
///            [--tags save,config]
/// custom list
/// custom remove <slug>   (с префиксом custom: или без)
fn cmd_custom(args: &[String]) -> i32 {
    use savesync_core::manifest::{Manifest, Os, PathRule};

    match args.first().map(String::as_str) {
        Some("add") => {
            let mut name: Option<String> = None;
            let mut paths: Vec<String> = Vec::new();
            let mut os = Os::Any;
            let mut tags: Vec<String> = Vec::new();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--path" => {
                        i += 1;
                        match args.get(i) {
                            Some(p) => paths.push(p.clone()),
                            None => return usage_custom(),
                        }
                    }
                    "--os" => {
                        i += 1;
                        os = match args.get(i).map(String::as_str) {
                            Some("windows") => Os::Windows,
                            Some("linux") => Os::Linux,
                            Some("any") => Os::Any,
                            _ => return usage_custom(),
                        };
                    }
                    "--tags" => {
                        i += 1;
                        match args.get(i) {
                            Some(t) => tags = t.split(',').map(|s| s.trim().to_string()).collect(),
                            None => return usage_custom(),
                        }
                    }
                    other if name.is_none() => name = Some(other.to_string()),
                    _ => return usage_custom(),
                }
                i += 1;
            }
            let Some(name) = name else {
                return usage_custom();
            };
            let rules: Vec<PathRule> = paths
                .into_iter()
                .map(|pattern| PathRule {
                    pattern,
                    os,
                    tags: tags.clone(),
                })
                .collect();
            let manifest = match Manifest::custom(&name, rules) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("невалидное правило: {e}");
                    return 1;
                }
            };
            let dir = custom_dir();
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("ошибка: {e}");
                return 1;
            }
            // Имя файла — slug без префикса custom: (':' недопустим в
            // именах файлов Windows).
            let file = dir.join(format!("{}.yaml", &manifest.slug["custom:".len()..]));
            let yaml = match manifest.to_yaml() {
                Ok(y) => y,
                Err(e) => {
                    eprintln!("ошибка сериализации: {e}");
                    return 1;
                }
            };
            if let Err(e) = std::fs::write(&file, yaml) {
                eprintln!("ошибка записи: {e}");
                return 1;
            }
            println!("добавлено: {} ({})", manifest.slug, file.display());
            0
        }
        Some("list") => {
            let dir = custom_dir();
            let (db, _) = savesync_core::manifest::load_manifest_tree(&dir);
            if db.is_empty() {
                println!("custom-правил нет");
            }
            for m in db.iter() {
                println!("{:<32} {} ({} путей)", m.slug, m.name, m.paths.len());
            }
            0
        }
        Some("remove") => {
            let Some(slug) = args.get(1) else {
                return usage_custom();
            };
            let plain = slug.strip_prefix("custom:").unwrap_or(slug);
            let file = custom_dir().join(format!("{plain}.yaml"));
            match std::fs::remove_file(&file) {
                Ok(()) => {
                    println!("удалено: custom:{plain}");
                    0
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("правило custom:{plain} не найдено");
                    1
                }
                Err(e) => {
                    eprintln!("ошибка: {e}");
                    1
                }
            }
        }
        _ => usage_custom(),
    }
}

fn usage_custom() -> i32 {
    eprintln!(
        "использование:\n  \
         custom add <name> --path <паттерн> [--path ...] [--os any|windows|linux] [--tags save,config]\n  \
         custom list\n  \
         custom remove <slug>"
    );
    2
}

/// remote setup fs <path> | remote setup yandex <token> | remote list
///
/// Каналы независимы (plan §3): можно настроить любой набор.
fn cmd_remote(args: &[String]) -> i32 {
    use savesync_core::config::YandexConfig;

    let path = config_path();
    let mut config = match Config::load_or_init(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config: {e}");
            return 1;
        }
    };

    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("setup"), Some("fs")) => {
            let Some(p) = args.get(2) else {
                return usage_remote();
            };
            config.backends.fs_path = Some(PathBuf::from(p));
        }
        (Some("setup"), Some("yandex")) => {
            let Some(token) = args.get(2) else {
                return usage_remote();
            };
            config.backends.yandex = Some(YandexConfig {
                token: token.clone(),
            });
        }
        (Some("setup"), Some("yandex-off")) => {
            config.backends.yandex = None;
        }
        (Some("setup"), Some("fs-off")) => {
            config.backends.fs_path = None;
        }
        (Some("list"), _) => {
            let fs = config
                .backends
                .fs_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".into());
            let ya = if config.backends.yandex.is_some() {
                "настроен"
            } else {
                "—"
            };
            println!("каналы синхронизации:\n  fs:          {fs}\n  yandex-disk: {ya}");
            return 0;
        }
        _ => return usage_remote(),
    }

    match config.save(&path) {
        Ok(()) => {
            println!("канал сохранён");
            0
        }
        Err(e) => {
            eprintln!("ошибка записи конфига: {e}");
            1
        }
    }
}

fn usage_remote() -> i32 {
    eprintln!(
        "использование:\n  \
         remote setup fs <путь>     — файловый канал (папка/USB)\n  \
         remote setup fs-off        — отключить файловый канал\n  \
         remote setup yandex <токен> — Яндекс.Диск (OAuth-токен)\n  \
         remote setup yandex-off    — отключить Яндекс.Диск\n  \
         remote list                — показать настроенные каналы"
    );
    2
}

fn load_runtime() -> Result<Runtime, String> {
    let cfg_path = config_path();
    let config = Config::load_or_init(&cfg_path).map_err(|e| format!("config: {e}"))?;
    Runtime::new(config, state_dir()).map_err(|e| format!("init: {e}"))
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
