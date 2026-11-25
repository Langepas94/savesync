use super::*;
use crate::manifest::parse_manifest_str;
use crate::scanner::SaveLocation;

/// Симуляция «устройства»: engine + корни + домашняя папка.
struct Device {
    engine: Engine<crate::backend::FsBackend>,
    home: PathBuf,
    roots: Roots,
}

struct World {
    dir: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "savesync-test-engine-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        World { dir }
    }

    fn backend(&self) -> crate::backend::FsBackend {
        crate::backend::FsBackend::new(self.dir.join("remote"))
    }

    fn device(&self, name: &str) -> Device {
        let home = self.dir.join(name).join("home");
        std::fs::create_dir_all(&home).unwrap();
        Device {
            engine: Engine::new(self.backend(), name, self.dir.join(name).join("state")),
            roots: Roots {
                home: Some(home.clone()),
                ..Roots::default()
            },
            home,
        }
    }
}

impl Drop for World {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn manifest() -> Manifest {
    parse_manifest_str(
        "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
    )
    .unwrap()
}

impl Device {
    fn write_save(&self, content: &str) {
        let dir = self.home.join("game/saves");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("slot1.sav"), content).unwrap();
    }

    fn read_save(&self) -> String {
        String::from_utf8(std::fs::read(self.home.join("game/saves/slot1.sav")).unwrap()).unwrap()
    }

    fn locations(&self) -> Vec<SaveLocation> {
        let resolved = self
            .home
            .join("game/saves")
            .to_string_lossy()
            .replace('\\', "/");
        vec![SaveLocation {
            location: None,
            pattern: "<home>/game/saves".into(),
            resolved: resolved.clone(),
            hits: vec![PathBuf::from(resolved)],
        }]
    }
}

#[test]
fn push_then_pull_to_second_device() {
    let world = World::new("two-devices");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    pc.write_save("pc progress");
    let outcome = pc.engine.push("test-game", &pc.locations()).unwrap();
    assert!(matches!(outcome, PushOutcome::Uploaded(_)));

    let outcome = deck
        .engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    assert!(matches!(outcome, PullOutcome::Applied { .. }));
    assert_eq!(deck.read_save(), "pc progress");

    // Повторный pull — уже актуально.
    assert_eq!(
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap(),
        PullOutcome::UpToDate
    );
}

#[test]
fn push_without_changes_is_unchanged() {
    let world = World::new("unchanged");
    let pc = world.device("pc");
    pc.write_save("v1");
    assert!(matches!(
        pc.engine.push("test-game", &pc.locations()).unwrap(),
        PushOutcome::Uploaded(_)
    ));
    assert_eq!(
        pc.engine.push("test-game", &pc.locations()).unwrap(),
        PushOutcome::Unchanged
    );
}

#[test]
fn foreign_lock_blocks_push_and_pull() {
    let world = World::new("locked");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    // PC залочил игру (игра запущена на PC).
    assert!(pc.engine.lock_game("test-game").unwrap());
    assert_eq!(
        deck.engine.foreign_lock("test-game").unwrap().as_deref(),
        Some("pc")
    );

    // Deck не может ни выгрузить, ни подтянуть.
    deck.write_save("deck progress");
    assert_eq!(
        deck.engine.push("test-game", &deck.locations()).unwrap(),
        PushOutcome::Locked("pc".into())
    );
    assert_eq!(
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap(),
        PullOutcome::Locked("pc".into())
    );

    // Свой лок push'у не мешает; повторный захват продлевает heartbeat.
    assert!(pc.engine.lock_game("test-game").unwrap());
    pc.write_save("pc progress");
    assert!(matches!(
        pc.engine.push("test-game", &pc.locations()).unwrap(),
        PushOutcome::Uploaded(_)
    ));

    // Снятие лока отпускает игру.
    pc.engine.unlock_game("test-game").unwrap();
    assert_eq!(deck.engine.foreign_lock("test-game").unwrap(), None);
    assert!(matches!(
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap(),
        PullOutcome::Applied { .. }
    ));
    assert_eq!(deck.read_save(), "pc progress");
}

#[test]
fn import_backup_registers_version() {
    let world = World::new("import");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    // PC делает переносимый бэкап (например, на флешку).
    pc.write_save("flash progress");
    let backup_dir = world.dir.join("usb");
    let file =
        crate::backup::create_backup("test-game", "pc", &pc.locations(), &backup_dir).unwrap();

    // deck импортирует его как новую версию в линии игры.
    let outcome = deck.engine.import_backup(&file).unwrap();
    let ImportOutcome::Registered(v) = outcome else {
        panic!("expected Registered, got {outcome:?}")
    };
    assert!(v.parent.is_none());
    assert_eq!(v.device, "pc(import)");

    // Версия видна в линии и применяется обычным pull.
    let pulled = deck
        .engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    assert!(matches!(pulled, PullOutcome::Applied { .. }));
    assert_eq!(deck.read_save(), "flash progress");

    // Повторный импорт того же файла — дубликат, версия не плодится.
    let again = deck.engine.import_backup(&file).unwrap();
    assert!(matches!(again, ImportOutcome::Duplicate(_)));
}

#[test]
fn two_way_sync_roundtrip() {
    let world = World::new("roundtrip");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    pc.write_save("from pc");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();

    deck.write_save("deck progress");
    deck.engine.push("test-game", &deck.locations()).unwrap();
    let outcome = pc.engine.pull(&m, &pc.roots, Platform::Windows).unwrap();
    assert!(matches!(outcome, PullOutcome::Applied { .. }));
    assert_eq!(pc.read_save(), "deck progress");
}

#[test]
fn diverged_pushes_conflict_under_ask() {
    let world = World::new("conflict");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    pc.write_save("base");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();

    // Обе стороны делают снапшот от одной базы — ветвление.
    pc.write_save("pc branch");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    deck.write_save("deck branch");
    deck.engine.push("test-game", &deck.locations()).unwrap();

    let outcome = deck
        .engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    match outcome {
        PullOutcome::Conflict(heads) => assert_eq!(heads.len(), 2),
        other => panic!("expected conflict, got {other:?}"),
    }
}

#[test]
fn latest_wins_policy_resolves_conflict() {
    let world = World::new("latest-wins");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    pc.write_save("base");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();

    pc.write_save("older branch");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2)); // разные created_at
    deck.write_save("newer branch");
    deck.engine.push("test-game", &deck.locations()).unwrap();

    // Переключаем политику на auto-latest в локальном индексе pc
    // (на deck LatestWins выбрал бы его же собственную голову —
    // для него состояние уже актуально).
    let mut idx = pc.engine.load_local().unwrap();
    idx.game_mut("test-game").policy = ConflictPolicy::LatestWins;
    pc.engine.save_local(&idx).unwrap();

    let outcome = pc.engine.pull(&m, &pc.roots, Platform::Windows).unwrap();
    assert!(matches!(outcome, PullOutcome::Applied { .. }));
    assert_eq!(pc.read_save(), "newer branch");
    // Проигравшая ветка не удалена — авто-выбор обратим.
    let idx = pc.engine.load_local().unwrap();
    assert_eq!(idx.game("test-game").unwrap().versions.len(), 4);
    assert_eq!(idx.game("test-game").unwrap().heads().len(), 1);
}

#[test]
fn pull_backs_up_before_overwrite() {
    let world = World::new("backup");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();

    pc.write_save("remote version");
    pc.engine.push("test-game", &pc.locations()).unwrap();

    deck.write_save("local precious");
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    assert_eq!(deck.read_save(), "remote version");

    // Локальное состояние сохранено в авто-бэкапе движка.
    let backups = deck.engine.state_dir.join("backups").join("test-game");
    let entries: Vec<_> = std::fs::read_dir(&backups).unwrap().collect();
    assert_eq!(entries.len(), 1);
    let backup_dir = entries[0].as_ref().unwrap().path();
    let found = find_file(&backup_dir, "slot1.sav").expect("backup must contain slot1.sav");
    assert_eq!(std::fs::read_to_string(found).unwrap(), "local precious");
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name()?.to_str()? == name {
            return Some(path);
        }
    }
    None
}

#[test]
fn rollback_to_specific_version() {
    let world = World::new("rollback");
    let pc = world.device("pc");
    let m = manifest();

    pc.write_save("v1 content");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    pc.write_save("v2 content");
    let v2 = match pc.engine.push("test-game", &pc.locations()).unwrap() {
        PushOutcome::Uploaded(v) => v,
        _ => panic!("expected upload"),
    };

    let idx = pc.engine.load_local().unwrap();
    let v1 = idx
        .game("test-game")
        .unwrap()
        .versions
        .iter()
        .find(|v| v.id != v2.id)
        .unwrap()
        .id
        .clone();

    let outcome = pc
        .engine
        .pull_version(&m, &pc.roots, Platform::Windows, &v1)
        .unwrap();
    assert!(matches!(outcome, PullOutcome::Applied { .. }));
    assert_eq!(pc.read_save(), "v1 content");
}
#[test]
fn unchanged_save_replicates_history_to_a_second_channel() {
    let world = World::new("second-channel");
    let pc = world.device("pc");
    pc.write_save("v1");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    pc.write_save("v2");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    let other = crate::backend::FsBackend::new(world.dir.join("other-remote"));
    // Upgrade from a state created before the persistent cache existed.
    std::fs::remove_dir_all(pc.engine.state_dir.join("snapshots")).unwrap();
    pc.engine.push("test-game", &pc.locations()).unwrap();
    let engine = Engine::new(other, "pc", &pc.engine.state_dir);
    assert_eq!(
        engine.push("test-game", &pc.locations()).unwrap(),
        PushOutcome::Unchanged
    );
    let remote = engine.load_remote().unwrap();
    let versions = &remote.game("test-game").unwrap().versions;
    assert_eq!(versions.len(), 2);
    for version in versions {
        assert!(engine
            .backend
            .get_snapshot("test-game", &version.id)
            .is_ok());
    }
}

#[test]
fn custom_key_syncs_on_windows_and_linux() {
    let world = World::new("custom-portable");
    let pc = world.device("pc");
    pc.write_save("custom save");
    pc.engine.push("custom:my-game", &pc.locations()).unwrap();
    let deck = world.device("deck");
    let mut m = manifest();
    m.slug = "custom:my-game".into();
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    assert_eq!(deck.read_save(), "custom save");
}

#[test]
fn conflict_choice_remains_resolved_on_next_sync() {
    let world = World::new("resolve-persisted");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = manifest();
    pc.write_save("base");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    pc.write_save("pc branch");
    let selected = match pc.engine.push("test-game", &pc.locations()).unwrap() {
        PushOutcome::Uploaded(v) => v.id,
        _ => panic!(),
    };
    deck.write_save("deck branch");
    deck.engine.push("test-game", &deck.locations()).unwrap();
    assert!(matches!(
        pc.engine.pull(&m, &pc.roots, Platform::Windows).unwrap(),
        PullOutcome::Conflict(_)
    ));
    assert_eq!(
        pc.engine
            .load_local()
            .unwrap()
            .game("test-game")
            .unwrap()
            .heads()
            .len(),
        2
    );
    pc.engine
        .pull_version(&m, &pc.roots, Platform::Windows, &selected)
        .unwrap();
    assert_eq!(
        pc.engine.push("test-game", &pc.locations()).unwrap(),
        PushOutcome::Unchanged
    );
    assert_eq!(
        pc.engine.pull(&m, &pc.roots, Platform::Windows).unwrap(),
        PullOutcome::UpToDate
    );
    assert_eq!(
        pc.engine
            .load_local()
            .unwrap()
            .game("test-game")
            .unwrap()
            .heads()
            .len(),
        1
    );
    deck.engine
        .pull(&m, &deck.roots, Platform::Windows)
        .unwrap();
    assert_eq!(deck.read_save(), "pc branch");
}

#[test]
fn restore_obeys_foreign_lock() {
    let world = World::new("restore-lock");
    let pc = world.device("pc");
    let deck = world.device("deck");
    pc.write_save("remote");
    let id = match pc.engine.push("test-game", &pc.locations()).unwrap() {
        PushOutcome::Uploaded(v) => v.id,
        _ => panic!(),
    };
    pc.engine.lock_game("test-game").unwrap();
    deck.write_save("precious");
    assert_eq!(
        deck.engine
            .pull_version(&manifest(), &deck.roots, Platform::Windows, &id)
            .unwrap(),
        PullOutcome::Locked("pc".into())
    );
    assert_eq!(deck.read_save(), "precious");
}

#[test]
fn unmapped_restore_does_not_mark_applied() {
    let world = World::new("unmapped");
    let pc = world.device("pc");
    let deck = world.device("deck");
    pc.write_save("remote");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    let mut m = manifest();
    m.paths[0].os = crate::manifest::Os::Windows;
    assert!(deck.engine.pull(&m, &deck.roots, Platform::Linux).is_err());
    assert!(deck.engine.applied_of("test-game").is_none());
}

#[test]
fn explicit_location_maps_different_platform_directories() {
    let world = World::new("platform-mapping");
    let pc = world.device("pc");
    let deck = world.device("deck");
    let m = parse_manifest_str("schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: '<home>/windows-save'\n    os: windows\n    location: saves\n  - pattern: '<home>/linux-save'\n    os: linux\n    location: saves\n").unwrap();
    std::fs::create_dir_all(pc.home.join("windows-save")).unwrap();
    std::fs::write(pc.home.join("windows-save/slot.sav"), "portable").unwrap();
    let db = crate::manifest::ManifestDb::from_entries(vec![m.clone()]);
    let games = crate::scanner::scan(&db, Platform::Windows, &pc.roots);
    pc.engine.push("test-game", &games[0].locations).unwrap();
    let mut m = m;
    m.paths[0].pattern = "<home>/updated-windows-path".into();
    deck.engine.pull(&m, &deck.roots, Platform::Linux).unwrap();
    assert_eq!(
        std::fs::read_to_string(deck.home.join("linux-save/slot.sav")).unwrap(),
        "portable"
    );
}

#[test]
fn missing_old_archive_does_not_block_latest_restore() {
    let world = World::new("missing-history");
    let pc = world.device("pc");
    let deck = world.device("deck");
    pc.write_save("old");
    let old = match pc.engine.push("test-game", &pc.locations()).unwrap() {
        PushOutcome::Uploaded(v) => v.id,
        _ => panic!(),
    };
    pc.write_save("current");
    pc.engine.push("test-game", &pc.locations()).unwrap();
    std::fs::remove_file(
        world
            .dir
            .join("remote/saves/test-game")
            .join(format!("{old}.zip")),
    )
    .unwrap();
    deck.engine
        .pull(&manifest(), &deck.roots, Platform::Windows)
        .unwrap();
    assert_eq!(deck.read_save(), "current");
}
