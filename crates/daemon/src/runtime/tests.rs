use super::*;
use savesync_core::config::BackendsConfig;
use savesync_core::manifest::parse_manifest_str;

struct World {
    dir: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "savesync-test-daemon-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        World { dir }
    }

    /// Симулирует «устройство»: своя home-папка с сейвом и state_dir.
    fn device(&self, name: &str) -> Runtime {
        let home = self.dir.join(name).join("home");
        std::fs::create_dir_all(home.join("game/saves")).unwrap();
        std::fs::write(home.join("game/saves/slot1.sav"), format!("save of {name}")).unwrap();

        let manifest = parse_manifest_str(
            "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
        )
        .unwrap();
        Runtime {
            config: Config {
                version: 1,
                device_id: name.into(),
                device_name: name.into(),
                manifests_dir: None,
                backends: BackendsConfig {
                    fs_path: Some(self.dir.join("remote")),
                    ..BackendsConfig::default()
                },
                backups: Default::default(),
            },
            manifests: ManifestDb::from_entries(vec![manifest]),
            platform: Platform::Windows,
            roots: Roots {
                home: Some(home),
                ..Roots::default()
            },
            state_dir: self.dir.join(name).join("state"),
            last_scan: Vec::new(),
            lan_server: None,
            serving: false,
        }
    }

    fn save_of(&self, device: &str) -> String {
        std::fs::read_to_string(self.dir.join(device).join("home/game/saves/slot1.sav")).unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn sync_results(payload: ResponsePayload) -> Vec<SyncResult> {
    match payload {
        ResponsePayload::Sync(p) => p.results,
        _ => panic!("expected sync payload"),
    }
}

#[test]
fn scan_finds_games_from_manifests() {
    let world = World::new("scan");
    let mut rt = world.device("pc");
    match rt.scan() {
        ResponsePayload::Scan(p) => {
            assert_eq!(p.found, 1);
            assert_eq!(p.games[0].game_key, "test-game");
        }
        _ => panic!("expected scan payload"),
    }
}

#[test]
fn sync_uploads_then_second_device_applies() {
    let world = World::new("sync");
    let mut pc = world.device("pc");
    let mut deck = world.device("deck");

    let results = sync_results(pc.sync(None));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, "uploaded");

    let results = sync_results(deck.sync(None));
    assert_eq!(results[0].outcome, "applied");
    assert_eq!(world.save_of("deck"), "save of pc");

    // Повторный sync — ничего не изменилось.
    let results = sync_results(deck.sync(None));
    assert_eq!(results[0].outcome, "up_to_date");
}

#[test]
fn sync_reports_conflict_without_resolving() {
    let world = World::new("conflict");
    let mut pc = world.device("pc");
    let mut deck = world.device("deck");

    pc.sync(None);
    deck.sync(None); // deck получил базу

    // Обе стороны расходятся от одной базы.
    std::fs::write(world.dir.join("pc/home/game/saves/slot1.sav"), "pc branch").unwrap();
    pc.sync(None);
    std::fs::write(
        world.dir.join("deck/home/game/saves/slot1.sav"),
        "deck branch",
    )
    .unwrap();
    deck.sync(None); // зальёт свою ветку

    let results = sync_results(deck.sync(Some("test-game")));
    assert_eq!(results[0].outcome, "conflict");
    // Конфликт не разрешён молча: локальный сейв на месте.
    assert_eq!(world.save_of("deck"), "deck branch");
}

#[test]
fn sync_unknown_game_returns_empty() {
    let world = World::new("unknown");
    let mut pc = world.device("pc");
    assert!(sync_results(pc.sync(Some("no-such-game"))).is_empty());
}

#[test]
fn custom_rules_from_state_dir_are_scanned() {
    let world = World::new("custom");
    let home = world.dir.join("pc/home");
    std::fs::create_dir_all(home.join("game/saves")).unwrap();
    std::fs::write(home.join("game/saves/slot1.sav"), "x").unwrap();

    // Custom-правило лежит в <state>/custom/<slug>.yaml.
    let state = world.dir.join("pc/state");
    let custom = state.join("custom");
    std::fs::create_dir_all(&custom).unwrap();
    std::fs::write(
        custom.join("my-game.yaml"),
        "schema: 1\nname: My Game\nslug: custom:my-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
    )
    .unwrap();

    let config = Config {
        version: 1,
        device_id: "pc".into(),
        device_name: "pc".into(),
        manifests_dir: None,
        backends: BackendsConfig::default(),
        backups: Default::default(),
    };
    let mut rt = Runtime::new(config, state).unwrap();
    // Runtime::new детектит корни реальной машины — в тесте подменяем.
    rt.platform = Platform::Windows;
    rt.roots = Roots {
        home: Some(home),
        ..Roots::default()
    };

    match rt.scan() {
        ResponsePayload::Scan(p) => {
            assert_eq!(p.found, 1);
            assert_eq!(p.games[0].game_key, "custom:my-game");
            assert_eq!(p.games[0].display_name, "My Game");
        }
        _ => panic!("expected scan payload"),
    }
}

#[test]
fn sync_rescans_globs_after_initial_scan() {
    let world = World::new("rescan-globs");
    let mut pc = world.device("pc");
    let manifest = parse_manifest_str("schema: 1\nname: Test Game\nslug: test-game\npaths:\n - pattern: '<home>/game/saves/*.sav'\n").unwrap();
    pc.manifests = ManifestDb::from_entries(vec![manifest]);
    pc.scan();
    pc.sync(None);
    std::fs::write(
        world.dir.join("pc/home/game/saves/new-slot.sav"),
        "new slot",
    )
    .unwrap();
    assert_eq!(sync_results(pc.sync(None))[0].outcome, "uploaded");
    assert_eq!(pc.last_scan[0].locations[0].hits.len(), 2);
}

#[test]
fn saving_config_updates_and_disables_live_lan_listener() {
    use savesync_core::{config::LanConfig, lan::LanBackend};
    let world = World::new("live-lan-config");
    let mut pc = world.device("pc");
    pc.start_lan().unwrap();
    let mut config = pc.config.clone();
    config.backends.lan = Some(LanConfig {
        port: 0,
        pin: "1111".into(),
        peers: vec![],
    });
    assert!(matches!(pc.save_config(config), ResponsePayload::Config(_)));
    let port = pc.lan_server.as_ref().unwrap().port;
    let address = format!("127.0.0.1:{port}");
    assert!(LanBackend::new(&address, "1111").ping().is_ok());
    let mut config = pc.config.clone();
    let lan = config.backends.lan.as_mut().unwrap();
    lan.port = port;
    lan.pin = "2222".into();
    assert!(matches!(pc.save_config(config), ResponsePayload::Config(_)));
    assert!(LanBackend::new(&address, "1111").ping().is_err());
    assert!(LanBackend::new(&address, "2222").ping().is_ok());
    let mut config = pc.config.clone();
    config.backends.lan = None;
    assert!(matches!(pc.save_config(config), ResponsePayload::Config(_)));
    assert!(pc.lan_server.is_none());
    assert!(std::net::TcpStream::connect(&address).is_err());
}
