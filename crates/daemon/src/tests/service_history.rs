use super::*;

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
    let history = deck.history("sample-install");
    assert_eq!(history.install_paths.len(), 1);
    assert_eq!(
        savesync_core::platform::paths::canonical_path(std::path::Path::new(
            &history.install_paths[0]
        )),
        savesync_core::platform::paths::canonical_path(&install)
    );
    assert_eq!(history.local_files.len(), 1);
    assert_eq!(
        savesync_core::platform::paths::canonical_path(std::path::Path::new(
            &history.local_files[0].path
        )),
        savesync_core::platform::paths::canonical_path(&save)
    );
    assert_eq!(
        sync_results(deck.sync(Some("sample-install")))[0].outcome,
        "uploaded"
    );
}
