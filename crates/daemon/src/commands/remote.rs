/// remote setup fs <path> | remote setup yandex <token> | remote list
///
/// Каналы независимы (plan §3): можно настроить любой набор.
use super::*;
pub(crate) fn cmd_remote(args: &[String]) -> i32 {
    use savesync_core::config::{GoogleConfig, YandexConfig};
    if args.first().is_some_and(|arg| arg == "login") {
        return super::authorization::login(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "logout") {
        return super::authorization::logout(&args[1..]);
    }

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

    super::authorization::save_config(config)
}

fn usage_remote() -> i32 {
    eprintln!(
        "использование:\n  remote login <google|yandex> — вход через браузер\n  remote logout <google|yandex> — отключить облако\n  \
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
