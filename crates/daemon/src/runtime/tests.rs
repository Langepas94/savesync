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
            shutdown_requested: false,
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
            base_manifests: ManifestDb::from_entries(vec![manifest]),
            manifests: ManifestDb::default(),
            platform: Platform::Windows,
            roots: Roots {
                home: Some(home),
                ..Roots::default()
            },
            state_dir: self.dir.join(name).join("state"),
            last_scan: Vec::new(),
            hidden_games: Default::default(),
            lan_server: None,
            serving: false,
            oauth_jobs: Default::default(),
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
    assert_eq!(results[0].outcome, "conflict");
    let ResponsePayload::History(history) = deck.history("test-game") else {
        panic!("expected history")
    };
    let remote = history.versions.iter().find(|v| v.device == "pc").unwrap();
    assert_eq!(
        sync_results(deck.restore("test-game", &remote.id))[0].outcome,
        "applied"
    );
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
    deck.sync(None);
    let ResponsePayload::History(history) = deck.history("test-game") else {
        panic!("expected history")
    };
    let remote = history.versions.iter().find(|v| v.device == "pc").unwrap();
    deck.restore("test-game", &remote.id);
    pc.sync(None);

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
fn first_sync_keeps_different_local_saves_for_user_choice() {
    let world = World::new("first-sync-conflict");
    let mut pc = world.device("pc");
    let mut deck = world.device("deck");
    assert_eq!(sync_results(pc.sync(None))[0].outcome, "uploaded");
    assert_eq!(sync_results(deck.sync(None))[0].outcome, "conflict");
    assert_eq!(world.save_of("deck"), "save of deck");
    let ResponsePayload::History(history) = deck.history("test-game") else {
        panic!("expected history")
    };
    assert_eq!(history.heads.len(), 2);
    assert_eq!(history.local_files.len(), 1);
    assert!(history.local_files[0].path.ends_with("slot1.sav"));
    let pc_version = history.versions.iter().find(|v| v.device == "pc").unwrap();
    assert_eq!(
        sync_results(deck.restore("test-game", &pc_version.id))[0].outcome,
        "applied"
    );
    assert_eq!(world.save_of("deck"), "save of pc");
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
    pc.base_manifests = ManifestDb::from_entries(vec![manifest]);
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
    let identity = world.dir.join("client-identity");
    pc.lan_server
        .as_ref()
        .unwrap()
        .open_pairing("123456")
        .unwrap();
    let offer = savesync_core::lan::pair(&identity, &address, "123456").unwrap();
    savesync_core::lan::confirm(&identity, &offer.peer_id).unwrap();
    let server_identity = pc.state_dir.join("lan-identity");
    let client_id = savesync_core::lan::peers(&server_identity).unwrap()[0]
        .peer
        .id
        .clone();
    savesync_core::lan::confirm(&server_identity, &client_id).unwrap();
    assert!(LanBackend::with_identity(&address, &identity)
        .ping()
        .is_ok());
    let mut config = pc.config.clone();
    let lan = config.backends.lan.as_mut().unwrap();
    lan.port = port;
    lan.pin = "2222".into();
    assert!(matches!(pc.save_config(config), ResponsePayload::Config(_)));
    assert!(LanBackend::with_identity(&address, &identity)
        .ping()
        .is_ok());
    let mut config = pc.config.clone();
    config.backends.lan = None;
    assert!(matches!(pc.save_config(config), ResponsePayload::Config(_)));
    assert!(pc.lan_server.is_none());
    assert!(std::net::TcpStream::connect(&address).is_err());
}

#[test]
fn discovers_ufs_from_secondary_library_and_pulls_without_local_saves() {
    let world = World::new("steam-empty");
    let mut pc = world.device("pc");
    let mut deck = world.device("deck");
    pc.platform = Platform::Linux;
    deck.platform = Platform::Linux;
    let steam = world.dir.join("steam");
    let secondary = world.dir.join("secondary");
    std::fs::create_dir_all(steam.join("steamapps")).unwrap();
    std::fs::create_dir_all(steam.join("appcache")).unwrap();
    std::fs::create_dir_all(secondary.join("steamapps")).unwrap();
    let library_path = secondary.to_string_lossy().replace('\\', "/");
    std::fs::write(
        steam.join("steamapps/libraryfolders.vdf"),
        format!("\"libraryfolders\" {{ \"1\" {{ \"path\" \"{library_path}\" }} }}"),
    )
    .unwrap();
    std::fs::write(
        secondary.join("steamapps/appmanifest_777001.acf"),
        "\"AppState\" { \"appid\" \"777001\" \"name\" \"Auto Game\" \"installdir\" \"Auto Game\" }",
    )
    .unwrap();
    std::fs::write(
        steam.join("appcache/appinfo.vdf"),
        include_bytes!("../../../core/tests/fixtures/appinfo-v40.vdf"),
    )
    .unwrap();
    pc.roots.steam_libraries = vec![steam.clone()];
    deck.roots.steam_libraries = vec![steam];
    pc.base_manifests = ManifestDb::default();
    deck.base_manifests = ManifestDb::default();
    deck.scan();
    assert_eq!(deck.last_scan.len(), 1);
    assert_eq!(deck.last_scan[0].game_key, "auto-game");
    assert!(deck.last_scan[0].locations.is_empty());
    let result = sync_results(deck.sync(Some("auto-game")));
    assert_eq!(result[0].outcome, "no_saves");
    assert!(!world.dir.join("remote/saves/auto-game").exists());
    let saves = pc.roots.home.as_ref().unwrap().join("ufs-saves");
    std::fs::create_dir_all(&saves).unwrap();
    std::fs::write(saves.join("slot.sav"), "from-pc").unwrap();
    assert_eq!(
        sync_results(pc.sync(Some("auto-game")))[0].outcome,
        "uploaded"
    );
    assert_eq!(
        sync_results(deck.sync(Some("auto-game")))[0].outcome,
        "applied"
    );
    assert_eq!(
        std::fs::read_to_string(deck.roots.home.as_ref().unwrap().join("ufs-saves/slot.sav"))
            .unwrap(),
        "from-pc"
    );
    assert!(deck
        .roots
        .steam_libraries
        .contains(&savesync_core::paths::canonical_path(&secondary)));
}

#[test]
fn installed_steam_game_resolves_save_rules_inside_its_library() {
    let world = World::new("steam-install-directory");
    let mut deck = world.device("deck");
    deck.platform = Platform::Linux;
    let steam = world.dir.join("sd card [games]");
    let save = steam.join("steamapps/common/Sample Install [test]/saves/slot.sav");
    std::fs::create_dir_all(save.parent().unwrap()).unwrap();
    std::fs::write(&save, b"portable progress").unwrap();
    std::fs::write(steam.join("steamapps/appmanifest_12345678.acf"),
        r#""AppState" { "appid" "12345678" "name" "Sample Install" "installdir" "Sample Install [test]" }"#).unwrap();
    deck.base_manifests = ManifestDb::from_entries(vec![parse_manifest_str(
        "schema: 1\nname: Sample Install\nslug: sample-install\nids:\n  steam: ['12345678']\npaths:\n  - pattern: '<base>/saves/*.sav'\n",
    ).unwrap()]);
    deck.roots.steam_libraries = vec![steam];
    deck.scan();
    let game = deck
        .last_scan
        .iter()
        .find(|game| game.game_key == "sample-install")
        .unwrap();
    assert_eq!(game.locations.len(), 1);
    assert_eq!(
        game.locations[0]
            .hits
            .iter()
            .map(|hit| savesync_core::paths::canonical_path(hit))
            .collect::<Vec<_>>(),
        [savesync_core::paths::canonical_path(&save)]
    );
    assert_eq!(
        sync_results(deck.sync(Some("sample-install")))[0].outcome,
        "uploaded"
    );
}

#[test]
fn empty_save_directory_does_not_create_snapshot() {
    let world = World::new("empty-directory");
    let mut pc = world.device("pc");
    std::fs::remove_file(world.dir.join("pc/home/game/saves/slot1.sav")).unwrap();
    let mut manifest = pc.base_manifests.get("test-game").unwrap().clone();
    manifest.slug = "custom:test-game".into();
    pc.base_manifests = ManifestDb::from_entries(vec![manifest]);
    assert_eq!(sync_results(pc.sync(None))[0].outcome, "no_saves");
    assert!(!world.dir.join("remote/saves").exists());
}

#[test]
fn installed_game_lists_relocated_profile_slot_in_history() {
    let world = World::new("steam-relocated-profile");
    let mut deck = world.device("deck");
    deck.platform = Platform::Linux;
    let steam = world.dir.join("steam");
    let apps = steam.join("steamapps");
    let install = apps.join("common/Sample Install");
    let save = apps.join("compatdata/12345678/pfx/drive_c/users/steamuser/AppData/Local/Publisher/Sample Install/profile/RUN00300");
    std::fs::create_dir_all(&install).unwrap();
    std::fs::create_dir_all(save.parent().unwrap()).unwrap();
    std::fs::write(&save, b"saved progress").unwrap();
    std::fs::write(apps.join("appmanifest_12345678.acf"),
        r#""AppState" { "appid" "12345678" "name" "Sample Install" "installdir" "Sample Install" }"#).unwrap();
    deck.base_manifests = ManifestDb::from_entries(vec![parse_manifest_str(
        "schema: 1\nname: Sample Install\nslug: sample-install\nids:\n  steam: ['12345678']\npaths:\n  - pattern: '<base>/saves/*.sav'\n",
    ).unwrap()]);
    deck.roots.steam_libraries = vec![steam];
    deck.scan();
    let ResponsePayload::History(history) = deck.history("sample-install") else {
        panic!("expected history")
    };
    assert_eq!(history.local_files.len(), 1);
    assert_eq!(
        savesync_core::paths::canonical_path(std::path::Path::new(&history.local_files[0].path)),
        savesync_core::paths::canonical_path(&save)
    );
    assert_eq!(
        sync_results(deck.sync(Some("sample-install")))[0].outcome,
        "uploaded"
    );
}

#[test]
fn blocked_manifest_never_uploads_save_files() {
    let world = World::new("blocked-discovery");
    let mut pc = world.device("pc");
    let mut manifest = pc.base_manifests.get("test-game").unwrap().clone();
    manifest.portability = savesync_core::manifest::Portability {
        status: savesync_core::manifest::PortabilityStatus::Blocked,
        reason: Some("machine bound".into()),
    };
    pc.base_manifests = ManifestDb::from_entries(vec![manifest]);
    assert_eq!(sync_results(pc.sync(None))[0].outcome, "skipped");
    assert!(!world.dir.join("remote/saves").exists());
}

#[test]
fn partial_ufs_keeps_curated_fallback_and_deduplicates_overlapping_files() {
    let world = World::new("ufs-fallback");
    let mut pc = world.device("pc");
    let steam = world.dir.join("steam");
    std::fs::create_dir_all(steam.join("steamapps")).unwrap();
    std::fs::create_dir_all(steam.join("appcache")).unwrap();
    std::fs::write(
        steam.join("steamapps/appmanifest_777001.acf"),
        "\"AppState\" { \"appid\" \"777001\" \"name\" \"Auto Game\" \"installdir\" \"Auto Game\" }",
    )
    .unwrap();
    std::fs::write(
        steam.join("appcache/appinfo.vdf"),
        include_bytes!("../../../core/tests/fixtures/appinfo-v40.vdf"),
    )
    .unwrap();
    pc.roots.steam_libraries = vec![steam.clone(), steam];
    let manifest=parse_manifest_str("schema: 1\nname: Canonical\nslug: canonical\nids:\n steam: ['777001']\npaths:\n - pattern: '<home>/ufs-saves'\n   location: legacy\n - pattern: '<home>/extra/*.sav'\n   location: extra\n").unwrap();
    pc.base_manifests = ManifestDb::from_entries(vec![manifest]);
    let home = pc.roots.home.as_ref().unwrap();
    for directory in ["ufs-saves", "extra"] {
        std::fs::create_dir_all(home.join(directory)).unwrap();
        std::fs::write(home.join(directory).join("a.sav"), directory).unwrap();
    }
    assert_eq!(
        sync_results(pc.sync(Some("canonical")))[0].outcome,
        "uploaded"
    );
    let game = &pc.last_scan[0];
    assert_eq!(game.game_key, "canonical");
    assert_eq!(
        game.locations
            .iter()
            .map(|location| location.hits.len())
            .sum::<usize>(),
        2
    );
    assert_eq!(pc.roots.steam_libraries.len(), 1);
    assert!(pc
        .manifests
        .get("canonical")
        .unwrap()
        .paths
        .iter()
        .any(|rule| rule.location.as_deref() == Some("extra")));
}

#[cfg(unix)]
#[test]
fn steam_symlink_aliases_are_one_canonical_library() {
    let world = World::new("steam-aliases");
    let mut pc = world.device("pc");
    let steam = world.dir.join("steam");
    std::fs::create_dir_all(steam.join("steamapps")).unwrap();
    let alias = world.dir.join("steam-alias");
    std::os::unix::fs::symlink(&steam, &alias).unwrap();
    pc.roots.steam_libraries = vec![steam.clone(), alias];
    pc.scan();
    assert_eq!(
        pc.roots.steam_libraries,
        vec![steam.canonicalize().unwrap()]
    );
}

#[cfg(unix)]
#[test]
fn unreadable_source_links_never_publish_a_readable_subset() {
    let world = World::new("source-link-failure");
    let mut pc = world.device("pc");
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("private.sav"), "outside").unwrap();
    std::os::unix::fs::symlink(&outside, world.dir.join("pc/home/game/saves/nested")).unwrap();
    assert_eq!(sync_results(pc.sync(Some("test-game")))[0].outcome, "error");
    assert!(!world.dir.join("remote/saves").exists());
}

#[test]
fn inaccessible_cloud_does_not_disable_filesystem_channel() {
    let world = World::new("oauth-independent");
    let mut runtime = world.device("pc");
    runtime.config.backends.google = Some(savesync_core::config::GoogleConfig {
        token: "keyring:invalid".into(),
    });
    let results = sync_results(runtime.sync(None));
    assert!(results.iter().any(|result| result.outcome == "uploaded"
        && result.detail.as_deref().unwrap_or("").contains("[fs]")));
    assert!(results.iter().any(|result| result.outcome == "error"
        && result
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("[google-drive]")));
}

#[test]
fn hidden_game_stays_excluded_until_restored() {
    let world = World::new("hidden-game");
    let mut rt = world.device("pc");
    rt.scan();
    assert!(matches!(
        rt.set_game_hidden("test-game", true),
        ResponsePayload::Scan(_)
    ));
    assert!(rt.last_scan.is_empty());
    assert!(sync_results(rt.sync(None)).is_empty());
    assert_eq!(world.save_of("pc"), "save of pc");
    let saved: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(rt.state_dir.join("hidden-games.json")).unwrap())
            .unwrap();
    assert_eq!(saved.get("test-game").unwrap(), "Test Game");
    let mut restarted = world.device("pc");
    restarted.hidden_games = saved;
    restarted.scan();
    assert!(restarted.last_scan.is_empty());
    assert!(matches!(
        restarted.set_game_hidden("test-game", false),
        ResponsePayload::Scan(_)
    ));
    assert_eq!(restarted.last_scan.len(), 1);
}

#[test]
fn readding_hidden_custom_game_replaces_paths_and_clears_hidden_state() {
    let world = World::new("readd-custom-game");
    let mut rt = world.device("pc");
    let home = rt.roots.home.clone().unwrap();
    let old = home.join("game/saves/slot1.sav");
    let new = home.join("new-save.sav");
    std::fs::write(&new, b"new save").unwrap();
    let input = |path: &std::path::Path| [path.to_string_lossy().into_owned()];
    let key = "custom:my-custom-game";

    assert!(rt.add_custom_game("My Custom Game", &input(&old)).is_ok());
    assert!(rt.add_custom_game("My Custom Game", &input(&new)).is_err());
    assert!(matches!(
        rt.set_game_hidden(key, true),
        ResponsePayload::Scan(_)
    ));
    assert!(rt
        .add_custom_game("My Custom Game", &input(&home.join("missing")))
        .is_err());
    assert!(rt.hidden_games.contains_key(key));

    assert!(matches!(
        rt.add_custom_game("My Custom Game", &input(&new)).unwrap(),
        ResponsePayload::Scan(_)
    ));
    let game = rt
        .last_scan
        .iter()
        .find(|game| game.game_key == key)
        .unwrap();
    assert_eq!(
        game.locations[0]
            .hits
            .iter()
            .map(|path| savesync_core::paths::canonical_path(path))
            .collect::<Vec<_>>(),
        [savesync_core::paths::canonical_path(&new)]
    );
    assert_eq!(std::fs::read_to_string(&old).unwrap(), "save of pc");
    assert!(!rt.hidden_games.contains_key(key));
    let restarted = Runtime::new(rt.config.clone(), rt.state_dir.clone()).unwrap();
    assert!(!restarted.hidden_games.contains_key(key));
    assert!(restarted.last_scan.iter().any(|game| game.game_key == key));
}

#[test]
fn portable_linux_game_and_windows_steam_game_share_progress_over_quic() {
    use savesync_core::{config::LanConfig, lan};
    let world = World::new("cross-platform-quic");
    let mut deck = world.device("deck");
    let mut pc = world.device("pc");
    let manifest = parse_manifest_str("schema: 1\nname: Sample Adventure\nslug: sample-adventure\naliases: [Adventure Shortcut]\nids:\n  steam: ['12345678']\npaths:\n  - pattern: '<documents>/Publisher/Sample Adventure/Saves/*.sav'\n    os: windows\n  - pattern: '<steamCompat>/12345678/pfx/drive_c/users/steamuser/Documents/Publisher/Sample Adventure/Saves/*.sav'\n    os: linux\n").unwrap();
    deck.base_manifests = ManifestDb::from_entries(vec![manifest.clone()]);
    pc.base_manifests = ManifestDb::from_entries(vec![manifest]);
    deck.platform = Platform::Linux;
    let install = world.dir.join("deck/home/Downloads/Sample Adventure");
    let prefix = world.dir.join("relocated-wine-prefix");
    let linux_save =
        prefix.join("drive_c/users/deck/My Documents/Publisher/Sample Adventure/Saves/slot.sav");
    let pc_docs = world.dir.join("pc/home/Documents");
    let windows_save = pc_docs.join("Publisher/Sample Adventure/Saves/slot.sav");
    let steam = world.dir.join("steam-pc");
    let lutris_config = world.dir.join("deck/home/.config/lutris/games");
    for directory in [
        install.as_path(),
        linux_save.parent().unwrap(),
        windows_save.parent().unwrap(),
        lutris_config.as_path(),
        steam.join("steamapps").as_path(),
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let exe = install.join("game.exe");
    std::fs::write(&exe, b"executable").unwrap();
    let lutris_yaml = serde_json::json!({"game": {"exe": exe, "prefix": prefix}});
    std::fs::write(
        lutris_config.join("sample-adventure.yml"),
        lutris_yaml.to_string(),
    )
    .unwrap();
    std::fs::write(&linux_save, b"progress from Linux").unwrap();
    std::fs::write(steam.join("steamapps/appmanifest_12345678.acf"),
        r#""AppState" { "appid" "12345678" "name" "Sample Adventure" "installdir" "Sample Adventure" }"#).unwrap();
    pc.roots.documents = Some(pc_docs);
    pc.roots.steam_libraries.push(steam);
    let added = deck
        .add_custom_game(
            "Adventure Shortcut",
            &[install.to_string_lossy().into_owned()],
        )
        .unwrap();
    let key = "sample-adventure";
    assert!(
        matches!(added, ResponsePayload::Scan(scan) if scan.games.iter().any(|game| game.game_key == key && game.locations == 1))
    );

    let server_identity = deck.state_dir.join("lan-identity");
    let pc_identity = pc.state_dir.join("lan-identity");
    let server =
        lan::LanServer::start_with_identity(deck.lan_store(), 0, &server_identity, false).unwrap();
    let address = format!("127.0.0.1:{}", server.port);
    server.open_pairing("123456").unwrap();
    let offer = lan::pair(&pc_identity, &address, "123456").unwrap();
    let peer = lan::peers(&server_identity).unwrap().remove(0);
    assert_eq!(peer.sas.as_deref(), Some(offer.sas.as_str()));
    lan::confirm(&pc_identity, &offer.peer_id).unwrap();
    lan::confirm(&server_identity, &peer.peer.id).unwrap();
    deck.config.backends = BackendsConfig {
        lan: Some(LanConfig {
            port: server.port,
            pin: String::new(),
            peers: vec![],
        }),
        ..Default::default()
    };
    pc.config.backends = BackendsConfig {
        lan: Some(LanConfig {
            port: 0,
            pin: String::new(),
            peers: vec![address],
        }),
        ..Default::default()
    };

    assert_eq!(sync_results(deck.sync(Some(key)))[0].outcome, "uploaded");
    let results = sync_results(pc.sync(Some(key)));
    assert!(
        results.iter().any(|result| result.outcome == "applied"),
        "{results:?}"
    );
    assert_eq!(
        std::fs::read(&windows_save).unwrap(),
        b"progress from Linux"
    );

    std::fs::write(&windows_save, b"progress from Windows").unwrap();
    let results = sync_results(pc.sync(Some(key)));
    assert!(
        results
            .iter()
            .all(|result| matches!(result.outcome.as_str(), "uploaded" | "up_to_date")),
        "{results:?}"
    );
    let results = sync_results(deck.sync(Some(key)));
    assert_eq!(results[0].outcome, "applied", "{results:?}");
    assert_eq!(
        std::fs::read(&linux_save).unwrap(),
        b"progress from Windows"
    );

    assert!(matches!(
        deck.set_game_hidden(key, true),
        ResponsePayload::Scan(_)
    ));
    let result = deck
        .add_custom_game(
            "Adventure Shortcut",
            &[linux_save.to_string_lossy().into_owned()],
        )
        .unwrap();
    assert!(
        matches!(result, ResponsePayload::Scan(scan) if scan.games.iter().any(|game| game.game_key == key && game.locations == 1))
    );
    assert!(
        matches!(deck.status(), ResponsePayload::Status(status) if status.games.iter().any(|game| game.game_key == key && game.versions > 0))
    );
}
