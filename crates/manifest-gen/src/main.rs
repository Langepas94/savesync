//! savesync-manifest-gen — генератор записей `savesync-manifests` из
//! локального `appinfo.vdf` (Steam). Формат выхода: schema v1
//! (docs/manifests-schema.md), один файл = одна игра, шардирование по
//! первому символу slug'а.
//!
//! Использование:
//!   savesync-manifest-gen --appinfo <path/to/appinfo.vdf> --out <repo-dir> [--filter <substr>] [--dry-run]
//!
//! Существующие файлы не перезаписываются (ручные правки комьюнити важнее
//! генерации). Каждая запись валидируется `Manifest::validate`.

use std::path::{Path, PathBuf};

use savesync_core::steam::appinfo;
use savesync_core::steam::ufs::manifest_from_entry;

#[derive(Debug, Default)]
pub struct GenStats {
    pub total_entries: usize,
    pub games_seen: usize,
    pub generated: usize,
    pub skipped_existing: usize,
    pub skipped_no_paths: usize,
    pub invalid: Vec<(String, Vec<String>)>,
}

/// Ядро генератора (тестируемо без CLI): appinfo.vdf -> дерево манифестов.
pub fn generate(
    appinfo_path: &Path,
    out_dir: &Path,
    filter: Option<&str>,
    dry_run: bool,
) -> std::io::Result<GenStats> {
    let entries = appinfo::load(appinfo_path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let mut stats = GenStats {
        total_entries: entries.len(),
        ..Default::default()
    };

    for entry in &entries {
        // Только игры с именем.
        if !entry
            .app_type()
            .map(|t| t.eq_ignore_ascii_case("game"))
            .unwrap_or(false)
        {
            continue;
        }
        let Some(name) = entry.display_name().map(str::to_string) else {
            continue;
        };
        if let Some(f) = filter {
            if !name.to_lowercase().contains(&f.to_lowercase()) {
                continue;
            }
        }
        stats.games_seen += 1;

        let Some(manifest) = manifest_from_entry(entry) else {
            stats.skipped_no_paths += 1;
            continue;
        };
        let slug = manifest.slug.clone();

        let errors = manifest.validate();
        if !errors.is_empty() {
            stats.invalid.push((slug, errors));
            continue;
        }

        let shard = shard_of(&slug);
        let path = out_dir
            .join("manifests")
            .join(shard)
            .join(format!("{slug}.yaml"));
        if path.exists() {
            stats.skipped_existing += 1;
            continue;
        }
        if !dry_run {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let yaml = serde_yaml::to_string(&manifest)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(&path, yaml)?;
        }
        stats.generated += 1;
    }
    Ok(stats)
}

/// Шард по первому символу slug'а (schema §2): буква или `0-9`.
pub fn shard_of(slug: &str) -> String {
    match slug.chars().next() {
        Some(c) if c.is_ascii_digit() => "0-9".to_string(),
        Some(c) => c.to_string(),
        None => "0-9".to_string(),
    }
}

fn main() {
    let mut appinfo_path: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut filter: Option<String> = None;
    let mut dry_run = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--appinfo" => appinfo_path = args.next().map(PathBuf::from),
            "--out" => out_dir = args.next().map(PathBuf::from),
            "--filter" => filter = args.next(),
            "--dry-run" => dry_run = true,
            _ => {
                eprintln!(
                    "savesync-manifest-gen {}\n  --appinfo <appinfo.vdf> --out <repo-dir> [--filter <substr>] [--dry-run]",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(2);
            }
        }
    }

    let (Some(appinfo_path), Some(out_dir)) = (appinfo_path, out_dir) else {
        eprintln!("нужны --appinfo и --out (см. --help-строку выше)");
        std::process::exit(2);
    };

    match generate(&appinfo_path, &out_dir, filter.as_deref(), dry_run) {
        Ok(stats) => {
            println!("записей appinfo: {}", stats.total_entries);
            println!("игр с именем:  {}", stats.games_seen);
            println!(
                "сгенерировано: {}{}",
                stats.generated,
                if dry_run { " (dry-run)" } else { "" }
            );
            println!("пропущено (уже есть): {}", stats.skipped_existing);
            println!("пропущено (нет путей): {}", stats.skipped_no_paths);
            if !stats.invalid.is_empty() {
                println!("невалидные ({}):", stats.invalid.len());
                for (slug, errors) in &stats.invalid {
                    println!("  {slug}: {}", errors.join("; "));
                }
            }
        }
        Err(e) => {
            eprintln!("ошибка: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_rules() {
        assert_eq!(shard_of("elden-ring"), "e");
        assert_eq!(shard_of("7-days-to-die"), "0-9");
    }

    // --- синтетический appinfo.vdf v40 с одной игрой с UFS ---

    fn cstr(s: &str) -> Vec<u8> {
        [s.as_bytes(), &[0]].concat()
    }

    fn kv_map_inline(key: &str, children: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8];
        b.extend(cstr(key));
        b.extend(children);
        b.push(8);
        b
    }

    fn kv_str(key: &str, val: &str) -> Vec<u8> {
        let mut b = vec![1u8];
        b.extend(cstr(key));
        b.extend(cstr(val));
        b
    }

    fn game_tree(appid: u32, name: &str) -> Vec<u8> {
        let _ = appid;
        let mut common = Vec::new();
        common.extend(kv_str("name", name));
        common.extend(kv_str("type", "Game"));
        let common = kv_map_inline("common", &common);

        let mut sf0 = Vec::new();
        sf0.extend(kv_str("root", "WinAppDataRoaming"));
        sf0.extend(kv_str("path", "TestGame"));
        sf0.extend(kv_map_inline("patterns", &kv_str("0", "*.sav")));
        let sf0 = kv_map_inline("0", &sf0);
        let savefiles = kv_map_inline("savefiles", &sf0);
        let ufs = kv_map_inline("ufs", &savefiles);

        let mut appinfo_inner = Vec::new();
        appinfo_inner.extend(common);
        appinfo_inner.extend(ufs);
        let appinfo = kv_map_inline("appinfo", &appinfo_inner);

        let mut root = Vec::new();
        root.extend(appinfo);
        root.push(8);
        root
    }

    fn appinfo_v40_with_game(appid: u32, name: &str) -> Vec<u8> {
        let tree = game_tree(appid, name);
        let mut record = Vec::new();
        let size = (4 + 4 + 8 + 20 + 4 + 20 + tree.len()) as u32;
        record.extend(appid.to_le_bytes());
        record.extend(size.to_le_bytes());
        record.extend(2u32.to_le_bytes());
        record.extend(1_700_000_000u32.to_le_bytes());
        record.extend(0u64.to_le_bytes());
        record.extend([0xAA; 20]);
        record.extend(1u32.to_le_bytes());
        record.extend([0xBB; 20]);
        record.extend(tree);

        let mut b = Vec::new();
        b.extend(0x07564428u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(record);
        b.extend(0u32.to_le_bytes());
        b
    }

    #[test]
    fn generates_manifest_from_appinfo() {
        let dir = std::env::temp_dir().join(format!("savesync-test-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let appinfo = dir.join("appinfo.vdf");
        std::fs::write(&appinfo, appinfo_v40_with_game(777001, "Test Game")).unwrap();

        let out = dir.join("repo");
        let stats = generate(&appinfo, &out, None, false).unwrap();
        assert_eq!(stats.games_seen, 1);
        assert_eq!(stats.generated, 1);

        let file = out.join("manifests/t/test-game.yaml");
        let yaml = std::fs::read_to_string(&file).unwrap();
        assert!(yaml.contains("slug: test-game"));
        assert!(yaml.contains("<winAppData>/TestGame/*.sav"));
        assert!(yaml.contains("<steamCompat>/777001/pfx/"));

        // Повторный прогон не перезаписывает (комьюнити-правки важнее).
        let stats2 = generate(&appinfo, &out, None, false).unwrap();
        assert_eq!(stats2.generated, 0);
        assert_eq!(stats2.skipped_existing, 1);

        // Фильтр по имени.
        let stats3 = generate(&appinfo, &out, Some("несуществующая"), true).unwrap();
        assert_eq!(stats3.games_seen, 0);

        std::fs::remove_dir_all(&dir).ok();
    }
}
