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

use savesync_core::platform::steam::appinfo;
use savesync_core::platform::steam::ufs::manifest_from_entry;

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
#[path = "tests/cli.rs"]
mod tests;
