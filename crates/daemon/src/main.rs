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
        Some("backup") => cmd_backup(&args[1..]),
        _ => {
            eprintln!(
                "savesync-daemon {}\nкоманды: serve | status | scan | sync [game_key] | custom <add|list|remove> | remote <setup|peer|list> | backup <create|list|import|dir>",
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
    use savesync_core::config::{GoogleConfig, YandexConfig};

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
        (Some("setup"), Some("google")) => {
            let Some(token) = args.get(2) else {
                return usage_remote();
            };
            config.backends.google = Some(GoogleConfig {
                token: token.clone(),
            });
        }
        (Some("setup"), Some("google-off")) => {
            config.backends.google = None;
        }
        (Some("setup"), Some("lan")) => {
            // remote setup lan <port> <pin>
            let (Some(port), Some(pin)) = (args.get(2), args.get(3)) else {
                return usage_remote();
            };
            let Ok(port) = port.parse::<u16>() else {
                eprintln!("некорректный порт: {port}");
                return 2;
            };
            let peers = config
                .backends
                .lan
                .take()
                .map(|l| l.peers)
                .unwrap_or_default();
            config.backends.lan = Some(savesync_core::config::LanConfig {
                port,
                pin: pin.clone(),
                peers,
            });
        }
        (Some("setup"), Some("lan-off")) => {
            config.backends.lan = None;
        }
        (Some("peer"), Some("add")) => {
            let Some(addr) = args.get(2) else {
                return usage_remote();
            };
            let Some(lan) = config.backends.lan.as_mut() else {
                eprintln!("сначала настройте LAN: remote setup lan <port> <pin>");
                return 1;
            };
            if !lan.peers.contains(addr) {
                lan.peers.push(addr.clone());
            }
        }
        (Some("peer"), Some("remove")) => {
            let Some(addr) = args.get(2) else {
                return usage_remote();
            };
            if let Some(lan) = config.backends.lan.as_mut() {
                lan.peers.retain(|p| p != addr);
            }
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
            let gg = if config.backends.google.is_some() {
                "настроен"
            } else {
                "—"
            };
            let lan = match &config.backends.lan {
                Some(l) => format!("порт {}, пиров: {}", l.port, l.peers.len()),
                None => "—".into(),
            };
            println!(
                "каналы синхронизации:\n  fs:           {fs}\n  yandex-disk:  {ya}\n  google-drive: {gg}\n  lan:          {lan}"
            );
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
         remote setup fs <путь>      — файловый канал (папка/USB)\n  \
         remote setup fs-off         — отключить файловый канал\n  \
         remote setup yandex <токен> — Яндекс.Диск (OAuth-токен)\n  \
         remote setup yandex-off     — отключить Яндекс.Диск\n  \
         remote setup google <токен> — Google Drive (OAuth access token)\n  \
         remote setup google-off     — отключить Google Drive\n  \
         remote setup lan <порт> <pin> — LAN-канал (сервер для пиров)\n  \
         remote setup lan-off        — отключить LAN\n  \
         remote peer add <host:port> — добавить пира (PIN общий)\n  \
         remote peer remove <host:port>\n  \
         remote list                 — показать настроенные каналы"
    );
    2
}

/// backup create <game_key> [dest] | backup list [game_key] |
/// backup import <файл> | backup dir <путь> | backup auto <часы>|auto-off
fn cmd_backup(args: &[String]) -> i32 {
    use savesync_core::backup;
    use savesync_core::engine::ImportOutcome;

    let path = config_path();
    let mut config = match Config::load_or_init(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config: {e}");
            return 1;
        }
    };

    match args.first().map(String::as_str) {
        Some("dir") => {
            let Some(p) = args.get(1) else {
                return usage_backup();
            };
            config.backups.dir = Some(PathBuf::from(p));
            match config.save(&path) {
                Ok(()) => {
                    println!("папка бэкапов: {p}");
                    0
                }
                Err(e) => {
                    eprintln!("ошибка записи конфига: {e}");
                    1
                }
            }
        }
        Some("auto") => {
            match args.get(1).map(String::as_str) {
                Some("off") => config.backups.auto_interval_hours = None,
                Some(h) => match h.parse::<u32>() {
                    Ok(h) if h > 0 => config.backups.auto_interval_hours = Some(h),
                    _ => return usage_backup(),
                },
                None => return usage_backup(),
            }
            match config.save(&path) {
                Ok(()) => {
                    println!("авто-бэкапы: {:?}", config.backups.auto_interval_hours);
                    0
                }
                Err(e) => {
                    eprintln!("ошибка записи конфига: {e}");
                    1
                }
            }
        }
        Some("list") => {
            let Some(dir) = &config.backups.dir else {
                eprintln!("папка бэкапов не задана (backup dir <путь>)");
                return 1;
            };
            match backup::list_backups(dir, args.get(1).map(String::as_str)) {
                Ok(list) => {
                    if list.is_empty() {
                        println!("бэкапов нет");
                    }
                    for b in list {
                        println!(
                            "{:<24} {} dev={} {}",
                            b.game_key,
                            b.created_at,
                            b.device_id,
                            b.path.display()
                        );
                    }
                    0
                }
                Err(e) => {
                    eprintln!("ошибка: {e}");
                    1
                }
            }
        }
        Some("create") => {
            let Some(game_key) = args.get(1) else {
                return usage_backup();
            };
            let dest = match args
                .get(2)
                .map(PathBuf::from)
                .or(config.backups.dir.clone())
            {
                Some(d) => d,
                None => {
                    eprintln!("куда? (backup create <game> <папка> или backup dir <путь>)");
                    return 1;
                }
            };
            let mut rt = match load_runtime() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("ошибка: {e}");
                    return 1;
                }
            };
            if rt.last_scan.is_empty() {
                rt.scan();
            }
            let Some(game) = rt.last_scan.iter().find(|g| &g.game_key == game_key) else {
                eprintln!("игра {game_key} не найдена сканером");
                return 1;
            };
            match backup::create_backup(
                &game.game_key,
                &rt.config.device_id,
                &game.locations,
                &dest,
            ) {
                Ok(file) => {
                    let keep = rt.config.backups.keep.unwrap_or(10) as usize;
                    let _ = backup::prune_backup_files(&dest, &game.game_key, keep);
                    println!("бэкап создан: {}", file.display());
                    0
                }
                Err(e) => {
                    eprintln!("ошибка: {e}");
                    1
                }
            }
        }
        Some("import") => {
            let Some(file) = args.get(1) else {
                return usage_backup();
            };
            let rt = match load_runtime() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("ошибка: {e}");
                    return 1;
                }
            };
            let engines = match rt.engines() {
                Ok(e) => e,
                Err(msg) => {
                    eprintln!("{msg}");
                    return 1;
                }
            };
            let file = PathBuf::from(file);
            let mut code = 0;
            for engine in &engines {
                match engine.import_backup(&file) {
                    Ok(ImportOutcome::Registered(v)) => {
                        println!("[{}] импортировано: {}", engine.backend_name(), v.id)
                    }
                    Ok(ImportOutcome::Duplicate(id)) => {
                        println!("[{}] дубликат: {}", engine.backend_name(), id)
                    }
                    Err(e) => {
                        eprintln!("[{}] ошибка: {e}", engine.backend_name());
                        code = 1;
                    }
                }
            }
            code
        }
        _ => usage_backup(),
    }
}

fn usage_backup() -> i32 {
    eprintln!(
        "использование:\n  \
         backup create <game_key> [папка] — переносимый бэкап (*.savesync-backup)\n  \
         backup list [game_key]           — список бэкапов в папке\n  \
         backup import <файл>             — зарегистрировать бэкап как новую версию\n  \
         backup dir <путь>                — папка бэкапов по умолчанию\n  \
         backup auto <часы> | auto off    — авто-бэкапы по расписанию (в serve)"
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
    // LAN-канал (plan §3): TCP-сервер для пиров поверх lan-store.
    if let Some(lan) = &runtime.config.backends.lan {
        let store = runtime.lan_store();
        let port = lan.port;
        let hash = savesync_core::lan::pin_hash(&lan.pin);
        std::thread::spawn(move || {
            if let Err(e) = savesync_core::lan::serve(store, port, hash) {
                eprintln!("lan server: {e}");
            }
        });
        println!("LAN-сервер: порт {port}");
    }
    let runtime = Arc::new(Mutex::new(runtime));
    // Авто-бэкапы по расписанию (plan §2.2): проверка раз в 30 минут.
    {
        let rt = runtime.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(30 * 60));
            let mut rt = match rt.lock() {
                Ok(rt) => rt,
                Err(_) => continue,
            };
            if rt.auto_backup_due() {
                match rt.run_auto_backup() {
                    Ok(n) if n > 0 => println!("авто-бэкап: {n} игр"),
                    Ok(_) => {}
                    Err(e) => eprintln!("авто-бэкап: {e}"),
                }
                rt.mark_auto_backup_done();
            }
        });
    }
    match ipc::serve(runtime, ipc::SOCKET_NAME) {
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
