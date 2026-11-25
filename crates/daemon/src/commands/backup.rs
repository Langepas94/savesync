/// backup create <game_key> [dest] | backup list [game_key] |
/// backup import <файл> | backup dir <путь> | backup auto <часы>|auto-off
use super::*;
pub(crate) fn cmd_backup(args: &[String]) -> i32 {
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
