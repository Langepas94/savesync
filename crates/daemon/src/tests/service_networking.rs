use super::*;

#[test]
fn saving_config_updates_and_disables_live_lan_listener() {
    use savesync_core::{
        config::LanConfig,
        lan::{self, LanAction, LanBackend},
    };
    let world = World::new("live-lan-config");
    let mut pc = world.device("pc");
    pc.start_lan().unwrap();
    let mut config = pc.config.clone();
    config.backends.lan = Some(LanConfig {
        port: 0,
        pin: "1111".into(),
        peers: vec![],
    });
    assert!(pc.save_config(config).is_ok());
    let port = pc.lan_server.as_ref().unwrap().port;
    let address = format!("127.0.0.1:{port}");
    let identity = world.dir.join("client-identity");
    let invitation = pc.lan_action(LanAction::Invite);
    let pin = invitation.invitation_pin.unwrap();
    let status = pc.lan_action(LanAction::Status);
    assert_eq!(status.invitation_pin.as_deref(), Some(pin.as_str()));
    let offer = lan::pair_named(&identity, &address, &pin, "Steam Deck").unwrap();
    lan::confirm(&identity, &offer.peer_id).unwrap();
    assert_eq!(
        lan::peers(&identity).unwrap()[0].peer.name.as_deref(),
        Some("pc")
    );
    let status = pc.lan_action(LanAction::Status);
    assert!(status.invitation_pin.is_none());
    assert_eq!(status.peers[0].peer.name.as_deref(), Some("Steam Deck"));
    assert_eq!(status.peers[0].sas.as_deref(), Some(offer.sas.as_str()));
    let client_id = status.peers[0].peer.id.clone();
    let status = pc.lan_action(LanAction::Confirm { peer_id: client_id });
    assert!(status.peers[0].confirmed);
    assert!(LanBackend::with_identity(&address, &identity)
        .ping()
        .is_ok());
    let peer = &lan::peers(&identity).unwrap()[0].peer;
    assert!(LanBackend::ping_peer(&identity, &address, &peer.public_key).is_ok());
    assert!(LanBackend::ping_peer(&identity, &address, &[0; 32]).is_err());
    let mut config = pc.config.clone();
    let lan = config.backends.lan.as_mut().unwrap();
    lan.port = port;
    lan.pin = "2222".into();
    assert!(pc.save_config(config).is_ok());
    assert!(LanBackend::with_identity(&address, &identity)
        .ping()
        .is_ok());
    let mut config = pc.config.clone();
    config.backends.lan = None;
    assert!(pc.save_config(config).is_ok());
    assert!(pc.lan_server.is_none());
    assert!(std::net::TcpStream::connect(&address).is_err());
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
    assert!(added
        .games
        .iter()
        .any(|game| game.game_key == key && game.locations == 1));

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

    assert!(deck.set_game_hidden(key, true).is_ok());
    let result = deck
        .add_custom_game(
            "Adventure Shortcut",
            &[linux_save.to_string_lossy().into_owned()],
        )
        .unwrap();
    assert!(result
        .games
        .iter()
        .any(|game| game.game_key == key && game.locations == 1));
    assert!(deck
        .status()
        .games
        .iter()
        .any(|game| game.game_key == key && game.versions > 0));
}
