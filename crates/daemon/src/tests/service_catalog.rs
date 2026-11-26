use super::*;

#[test]
fn scan_finds_games_from_manifests() {
    let world = World::new("scan");
    let mut rt = world.device("pc");
    let p = rt.scan();
    assert_eq!(p.found, 1);
    assert_eq!(p.games[0].game_key, "test-game");
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

    let p = rt.scan();
    assert_eq!(p.found, 1);
    assert_eq!(p.games[0].game_key, "custom:my-game");
    assert_eq!(p.games[0].display_name, "My Game");
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
            .map(|hit| savesync_core::platform::paths::canonical_path(hit))
            .collect::<Vec<_>>(),
        [savesync_core::platform::paths::canonical_path(&save)]
    );
    assert_eq!(
        sync_results(deck.sync(Some("sample-install")))[0].outcome,
        "uploaded"
    );
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
        include_bytes!("../../tests/fixtures/appinfo-v40.vdf"),
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

#[test]
fn hidden_game_stays_excluded_until_restored() {
    let world = World::new("hidden-game");
    let mut rt = world.device("pc");
    rt.scan();
    assert!(rt.set_game_hidden("test-game", true).is_ok());
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
    assert!(restarted.set_game_hidden("test-game", false).is_ok());
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
    assert!(rt.set_game_hidden(key, true).is_ok());
    assert!(rt
        .add_custom_game("My Custom Game", &input(&home.join("missing")))
        .is_err());
    assert!(rt.hidden_games.contains_key(key));

    assert!(rt.add_custom_game("My Custom Game", &input(&new)).is_ok());
    let game = rt
        .last_scan
        .iter()
        .find(|game| game.game_key == key)
        .unwrap();
    assert_eq!(
        game.locations[0]
            .hits
            .iter()
            .map(|path| savesync_core::platform::paths::canonical_path(path))
            .collect::<Vec<_>>(),
        [savesync_core::platform::paths::canonical_path(&new)]
    );
    assert_eq!(std::fs::read_to_string(&old).unwrap(), "save of pc");
    assert!(!rt.hidden_games.contains_key(key));
    let restarted = Runtime::new(rt.config.clone(), rt.state_dir.clone()).unwrap();
    assert!(!restarted.hidden_games.contains_key(key));
    assert!(restarted.last_scan.iter().any(|game| game.game_key == key));
}
