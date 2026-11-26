//! Сканер установленных игр: `savesync-manifests` + корни системы ->
//! найденные игры. Steam appinfo (парсер — отдельная задача M1) дополняет
//! базу appid→slug и Proton-маппинг, но сканер от него не зависит.
//!
//! Чистые функции: сканирование принимает платформу и корни явно, поэтому
//! матрица кейсов Win/Linux прогоняется в тестах на любой ОС.

use std::path::PathBuf;

use crate::catalog::manifest::ManifestDb;
use crate::platform::paths::{resolve_pattern_multi, Platform, Roots};

/// Найденный на диске путь сейвов одной игры.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveLocation {
    /// Stable logical save-location ID from the game manifest.
    pub location: Option<String>,
    /// Исходный паттерн из манифеста.
    pub pattern: String,
    /// Резолвнутый glob-паттерн.
    pub resolved: String,
    /// Реально существующие пути, попавшие под паттерн.
    pub hits: Vec<PathBuf>,
}

/// Игра, найденная сканером на этом устройстве.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundGame {
    /// Store-агностичный ключ = slug из `savesync-manifests`.
    pub game_key: String,
    /// Имя из манифеста, для UI.
    pub display_name: String,
    /// Первый известный Steam appid (если есть) — для Proton-маппинга и UI.
    pub steam_appid: Option<String>,
    /// Переносимость сейвов (варн/блок-лист) — UI показывает предупреждение.
    pub portable: crate::catalog::manifest::PortabilityStatus,
    pub locations: Vec<SaveLocation>,
}

/// Сканирует систему по базе манифестов.
///
/// Для каждой игры резолвятся `paths`, подходящие под платформу
/// (`os:`), затем glob проверяет их существование. Игра попадает
/// в результат, если найден хотя бы один реальный путь.
pub fn scan(db: &ManifestDb, platform: Platform, roots: &Roots) -> Vec<FoundGame> {
    let mut found = Vec::new();

    for manifest in db.iter() {
        let mut locations = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for rule in manifest.paths_for(platform) {
            for resolved in resolve_pattern_multi(&rule.pattern, roots) {
                let resolved = crate::platform::paths::canonical_pattern(&resolved);
                let mut hits = Vec::new();
                let base = crate::storage::snapshot::location_base(&resolved);
                if let Ok(paths) = glob::glob(&resolved) {
                    for path in paths {
                        let hit = match path {
                            Ok(hit) => hit,
                            Err(error) => error.path().to_path_buf(),
                        };
                        let mut files = Vec::new();
                        if crate::platform::paths::validate_source_path(&base, &hit)
                            .and_then(|_| collect_regular_files(&base, &hit, &mut files))
                            .is_ok()
                        {
                            hits.extend(files);
                        } else {
                            // Preserve the failing path so strict snapshot traversal reports
                            // the error instead of publishing the readable subset as success.
                            hits.push(hit);
                        }
                    }
                }
                hits.sort();
                hits.dedup();
                hits.retain(|path| seen.insert(path.clone()));
                if !hits.is_empty() {
                    locations.push(SaveLocation {
                        location: rule.location.clone(),
                        pattern: rule.pattern.clone(),
                        resolved,
                        hits,
                    });
                }
            }
        }

        if !locations.is_empty() {
            found.push(FoundGame {
                game_key: manifest.slug.clone(),
                display_name: manifest.name.clone(),
                steam_appid: manifest.ids.steam.first().cloned(),
                portable: manifest.portability.status,
                locations,
            });
        }
    }

    found.sort_by(|a, b| a.game_key.cmp(&b.game_key));
    found
}

fn collect_regular_files(
    base: &std::path::Path,
    path: &std::path::Path,
    out: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    crate::platform::paths::validate_source_path(base, path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_file() {
        if !crate::storage::backup::is_backup_archive(path)
            && !path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("steam_autocloud.vdf"))
        {
            out.push(path.to_path_buf());
        }
    } else if metadata.is_dir() {
        for entry in std::fs::read_dir(path)? {
            collect_regular_files(base, &entry?.path(), out)?;
        }
    }
    Ok(())
}

/// Include installed/previously synced games and explicit custom rules even
/// before their first save is created. Empty locations must never be uploaded.
pub fn scan_catalog(
    db: &ManifestDb,
    platform: Platform,
    roots: &Roots,
    known: &std::collections::HashSet<String>,
) -> Vec<FoundGame> {
    let mut games = scan(db, platform, roots);
    for manifest in db.iter() {
        if (known.contains(&manifest.slug) || manifest.slug.starts_with("custom:"))
            && !games.iter().any(|game| game.game_key == manifest.slug)
        {
            games.push(FoundGame {
                game_key: manifest.slug.clone(),
                display_name: manifest.name.clone(),
                steam_appid: manifest.ids.steam.first().cloned(),
                portable: manifest.portability.status,
                locations: vec![],
            });
        }
    }
    games.sort_by(|a, b| a.game_key.cmp(&b.game_key));
    games
}

#[cfg(test)]
#[path = "../tests/catalog/scanner.rs"]
mod tests;
