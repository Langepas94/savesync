use super::*;
fn custom_dir() -> PathBuf {
    state_dir().join("custom")
}

/// custom add <name> --path <p> [--path ...] [--os any|windows|linux]
///            [--tags save,config]
/// custom list
/// custom remove <slug>   (с префиксом custom: или без)
pub(crate) fn cmd_custom(args: &[String]) -> i32 {
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
                    location: None,
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
            if !savesync_core::game_key::is_valid_slug(plain) {
                eprintln!("невалидный slug custom-правила");
                return 2;
            }
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
