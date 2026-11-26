use super::*;

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
    let history = deck.history("test-game");
    let remote = history.versions.iter().find(|v| v.device == "pc").unwrap();
    assert_eq!(remote.channel.as_deref(), Some("fs"));
    assert_eq!(remote.device_name.as_deref(), Some("pc"));
    assert_eq!(remote.file_count, Some(1));
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
    let history = deck.history("test-game");
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
    let history = deck.history("test-game");
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
        include_bytes!("../../tests/fixtures/appinfo-v40.vdf"),
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
        .contains(&savesync_core::platform::paths::canonical_path(&secondary)));
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
fn blocked_manifest_never_uploads_save_files() {
    let world = World::new("blocked-discovery");
    let mut pc = world.device("pc");
    let mut manifest = pc.base_manifests.get("test-game").unwrap().clone();
    manifest.portability = savesync_core::catalog::manifest::Portability {
        status: savesync_core::catalog::manifest::PortabilityStatus::Blocked,
        reason: Some("machine bound".into()),
    };
    pc.base_manifests = ManifestDb::from_entries(vec![manifest]);
    assert_eq!(sync_results(pc.sync(None))[0].outcome, "skipped");
    assert!(!world.dir.join("remote/saves").exists());
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
