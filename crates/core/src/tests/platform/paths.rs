use super::*;

fn windows_roots() -> Roots {
    Roots {
        home: Some(PathBuf::from(r"C:\Users\gamer")),
        win_appdata: Some(PathBuf::from(r"C:\Users\gamer\AppData\Roaming")),
        win_local_appdata: Some(PathBuf::from(r"C:\Users\gamer\AppData\Local")),
        win_saved_games: Some(PathBuf::from(r"C:\Users\gamer\Saved Games")),
        documents: Some(PathBuf::from(r"C:\Users\gamer\Documents")),
        ..Roots::default()
    }
}

fn linux_roots() -> Roots {
    Roots {
        home: Some(PathBuf::from("/home/deck")),
        xdg_data: Some(PathBuf::from("/home/deck/.local/share")),
        xdg_config: Some(PathBuf::from("/home/deck/.config")),
        documents: Some(PathBuf::from("/home/deck/Documents")),
        steam_libraries: vec![
            PathBuf::from("/home/deck/.local/share/Steam"),
            PathBuf::from("/run/media/mmcblk0p1"),
        ],
        ..Roots::default()
    }
}

#[test]
fn windows_placeholders() {
    let r = windows_roots();
    assert_eq!(
        resolve_pattern("<winAppData>/SomeGame/saves/*.sav", &r).as_deref(),
        Some("C:/Users/gamer/AppData/Roaming/SomeGame/saves/*.sav")
    );
    assert_eq!(
        resolve_pattern("<winSavedGames>/Studio/Game", &r).as_deref(),
        Some("C:/Users/gamer/Saved Games/Studio/Game")
    );
    assert_eq!(
        resolve_pattern("<home>/Documents/My Games/Game", &r).as_deref(),
        Some("C:/Users/gamer/Documents/My Games/Game")
    );
}

#[test]
fn linux_placeholders() {
    let r = linux_roots();
    assert_eq!(
        resolve_pattern("<xdgData>/game/saves", &r).as_deref(),
        Some("/home/deck/.local/share/game/saves")
    );
    assert_eq!(
        resolve_pattern("<home>/.local/share/Steam", &r).as_deref(),
        Some("/home/deck/.local/share/Steam")
    );
}

#[test]
fn steam_compat_expands_per_library() {
    let r = linux_roots();
    let resolved = resolve_pattern_multi(
        "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing",
        &r,
    );
    assert_eq!(
        resolved,
        vec![
            "/home/deck/.local/share/Steam/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing",
            "/run/media/mmcblk0p1/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing",
        ]
    );
    // На Windows без Steam-библиотек Proton-маппинга нет.
    assert!(resolve_pattern_multi("<steamCompat>/1/pfx", &windows_roots()).is_empty());
}

#[test]
fn cross_platform_mismatch_returns_none() {
    assert_eq!(resolve_pattern("<winAppData>/Game", &linux_roots()), None);
    assert_eq!(resolve_pattern("<xdgData>/game", &windows_roots()), None);
}

#[test]
fn unknown_placeholder_is_skipped() {
    let r = windows_roots();
    assert!(resolve_pattern_multi("<someFuture>/x", &r).is_empty());
}

#[test]
fn plain_path_passes_through() {
    let r = windows_roots();
    assert_eq!(
        resolve_pattern("D:/Games/saves", &r).as_deref(),
        Some("D:/Games/saves")
    );
}
