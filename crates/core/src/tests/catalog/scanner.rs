use super::*;
use crate::catalog::manifest::{parse_manifest_str, ManifestDb, PortabilityStatus};

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
fn backup_archives_inside_a_save_folder_are_not_synced() {
    let fx = Fixture::new("nested-backup");
    fx.touch("saves/current.sav");
    fx.touch("saves/archive/previous.savesync-backup");
    let db = db_from(&[
        "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: '<home>/saves'\n",
    ]);
    let roots = Roots {
        home: Some(fx.path("")),
        ..Roots::default()
    };
    let found = scan(&db, Platform::current(), &roots);
    let hits = &found[0].locations[0].hits;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file_name().unwrap(), "current.sav");
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
