//! Сканер установленных игр: `savesync-manifests` + корни системы ->
//! найденные игры. Steam appinfo (парсер — отдельная задача M1) дополняет
//! базу appid→slug и Proton-маппинг, но сканер от него не зависит.
//!
//! Чистые функции: сканирование принимает платформу и корни явно, поэтому
//! матрица кейсов Win/Linux прогоняется в тестах на любой ОС.

use std::path::PathBuf;

use crate::manifest::ManifestDb;
use crate::paths::{resolve_pattern_multi, Platform, Roots};

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
    pub portable: crate::manifest::PortabilityStatus,
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
                let resolved = crate::paths::canonical_pattern(&resolved);
                let mut hits = Vec::new();
                let base = crate::snapshot::location_base(&resolved);
                if let Ok(paths) = glob::glob(&resolved) {
                    for path in paths {
                        let hit = match path {
                            Ok(hit) => hit,
                            Err(error) => error.path().to_path_buf(),
                        };
                        let mut files = Vec::new();
                        if crate::paths::validate_source_path(&base, &hit)
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
    crate::paths::validate_source_path(base, path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_file() {
        if !path
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
mod tests {
    use super::*;
    use crate::manifest::{parse_manifest_str, ManifestDb, PortabilityStatus};

    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "savesync-test-scan-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Fixture { dir }
        }

        fn touch(&self, rel: &str) {
            let p = self.dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"save").unwrap();
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.dir.join(rel)
        }

        fn display(&self) -> String {
            self.dir.to_string_lossy().replace('\\', "/")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn db_from(yamls: &[&str]) -> ManifestDb {
        ManifestDb::from_entries(
            yamls
                .iter()
                .map(|y| parse_manifest_str(y).unwrap())
                .collect(),
        )
    }

    #[test]
    fn finds_game_on_windows_layout() {
        let fx = Fixture::new("win");
        fx.touch("AppData/Roaming/TestGame/saves/slot1.sav");

        let db = db_from(&[
            "schema: 1\nname: Test Game\nslug: test-game\nids:\n  steam: [\"111\"]\npaths:\n  - pattern: \"<winAppData>/TestGame/saves/*.sav\"\n    os: windows\n",
        ]);
        let roots = Roots {
            home: Some(fx.path("")),
            win_appdata: Some(fx.path("AppData/Roaming")),
            ..Roots::default()
        };

        let found = scan(&db, Platform::Windows, &roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].game_key, "test-game");
        assert_eq!(found[0].display_name, "Test Game");
        assert_eq!(found[0].steam_appid.as_deref(), Some("111"));
        assert_eq!(found[0].portable, PortabilityStatus::Ok);
        assert_eq!(found[0].locations.len(), 1);
        assert_eq!(found[0].locations[0].hits.len(), 1);
    }

    #[test]
    fn finds_game_on_linux_layout() {
        let fx = Fixture::new("linux");
        fx.touch(".config/TestGame/saves/slot1.sav");

        let db = db_from(&[
            "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<xdgConfig>/TestGame/saves/*.sav\"\n    os: linux\n",
        ]);
        let roots = Roots {
            home: Some(fx.path("")),
            xdg_config: Some(fx.path(".config")),
            ..Roots::default()
        };

        let found = scan(&db, Platform::Linux, &roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].game_key, "test-game");
    }

    #[test]
    fn finds_proton_saves_via_steam_compat() {
        let fx = Fixture::new("proton");
        fx.touch("steamlib/steamapps/compatdata/111/pfx/drive_c/users/steamuser/AppData/Roaming/TestGame/save.sav");

        let db = db_from(&[
            "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<steamCompat>/111/pfx/drive_c/users/steamuser/AppData/Roaming/TestGame\"\n    os: linux\n",
        ]);
        let roots = Roots {
            steam_libraries: vec![fx.path("steamlib")],
            ..Roots::default()
        };

        let found = scan(&db, Platform::Linux, &roots);
        assert_eq!(found.len(), 1);
        assert!(found[0].locations[0].resolved.contains("compatdata/111"));
    }

    #[test]
    fn os_filtering_skips_other_platform() {
        let fx = Fixture::new("filter");
        fx.touch("AppData/Roaming/TestGame/save.sav");

        let db = db_from(&[
            "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<winAppData>/TestGame/save.sav\"\n    os: windows\n",
        ]);
        let roots = Roots {
            home: Some(fx.path("")),
            xdg_data: Some(fx.path(".local/share")),
            ..Roots::default()
        };

        assert!(scan(&db, Platform::Linux, &roots).is_empty());
    }

    #[test]
    fn portability_warn_propagates() {
        let fx = Fixture::new("warn");
        fx.touch("game/save.sav");

        let db = db_from(&[
            "schema: 1\nname: Locked Game\nslug: locked-game\npaths:\n  - pattern: \"<home>/game/save.sav\"\nportability:\n  status: blocked\n  reason: \"machine-locked secure_save\"\n",
        ]);
        let roots = Roots {
            home: Some(fx.path("")),
            ..Roots::default()
        };

        let found = scan(&db, Platform::Linux, &roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].portable, PortabilityStatus::Blocked);
    }

    #[test]
    fn missing_saves_mean_game_not_found() {
        let fx = Fixture::new("empty");
        let _ = fx.display();
        let db = db_from(&[
            "schema: 1\nname: Absent Game\nslug: absent-game\npaths:\n  - pattern: \"<home>/NoSuchGame/saves\"\n",
        ]);
        let roots = Roots {
            home: Some(fx.path("")),
            ..Roots::default()
        };

        assert!(scan(&db, Platform::Linux, &roots).is_empty());
    }

    #[test]
    fn multiple_games_sorted_by_key() {
        let fx = Fixture::new("multi");
        fx.touch("game-b/save.sav");
        fx.touch("game-a/save.sav");

        let db = db_from(&[
            "schema: 1\nname: Game B\nslug: game-b\npaths:\n  - pattern: \"<home>/game-b/save.sav\"\n",
            "schema: 1\nname: Game A\nslug: game-a\npaths:\n  - pattern: \"<home>/game-a/save.sav\"\n",
        ]);
        let roots = Roots {
            home: Some(fx.path("")),
            ..Roots::default()
        };

        let found = scan(&db, Platform::Linux, &roots);
        let keys: Vec<&str> = found.iter().map(|g| g.game_key.as_str()).collect();
        assert_eq!(keys, vec!["game-a", "game-b"]);
    }
}
