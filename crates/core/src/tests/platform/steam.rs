use super::*;
use crate::manifest::{parse_manifest_str, ManifestDb};

const MANIFEST: &str = r#"
schema: 1
name: Elden Ring
slug: elden-ring
ids:
  steam: ["1245620"]
paths:
  - pattern: "<winAppData>/EldenRing"
    os: windows
"#;

#[test]
fn parses_modern_libraryfolders() {
    let vdf = r#"
"libraryfolders"
{
    "0" { "path" "C:\\Steam" }
    "1" { "path" "D:\\SteamLibrary" }
    "contentstatsid" "123"
}
"#;
    let roots = parse_library_folders(vdf);
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], PathBuf::from("C:\\Steam"));
    assert_eq!(roots[1], PathBuf::from("D:\\SteamLibrary"));
}

#[test]
fn parses_legacy_libraryfolders() {
    let vdf = "\"libraryfolders\"\n{\n\"0\" \"C:\\\\Steam\"\n\"1\" \"/mnt/ssd/steam\"\n}\n";
    let roots = parse_library_folders(vdf);
    assert_eq!(roots[1], PathBuf::from("/mnt/ssd/steam"));
}

#[test]
fn scan_library_reads_appmanifests_and_compatdata() {
    let dir = std::env::temp_dir().join(format!("savesync-test-steam-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let steamapps = dir.join("steamapps");
    std::fs::create_dir_all(&steamapps).unwrap();
    std::fs::write(
            steamapps.join("appmanifest_1245620.acf"),
            "\"AppState\"\n{\n\"appid\" \"1245620\"\n\"name\" \"ELDEN RING\"\n\"installdir\" \"ELDEN RING\"\n}\n",
        )
        .unwrap();
    std::fs::write(steamapps.join("appmanifest_broken.acf"), "garbage").unwrap();
    std::fs::create_dir_all(steamapps.join("compatdata").join("1245620")).unwrap();

    let lib = scan_library(&dir);
    assert_eq!(lib.apps.len(), 1);
    let app = &lib.apps[0];
    assert_eq!(app.appid, 1245620);
    assert_eq!(app.name, "ELDEN RING");
    assert!(app.compat_prefix.is_some());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn matches_by_steam_id_then_by_slug() {
    let db = ManifestDb::from_entries(vec![
        parse_manifest_str(MANIFEST).unwrap(),
        parse_manifest_str(
            "schema: 1\nname: Hades\nslug: hades\npaths:\n  - pattern: \"<home>/hades\"\n",
        )
        .unwrap(),
    ]);
    let apps = vec![
        SteamApp {
            appid: 1245620,
            name: "ELDEN RING".into(),
            installdir: "ELDEN RING".into(),
            library: PathBuf::from("/lib"),
            compat_prefix: None,
        },
        // Нет ids.steam в манифесте — матч по slug имени.
        SteamApp {
            appid: 1145360,
            name: "Hades".into(),
            installdir: "Hades".into(),
            library: PathBuf::from("/lib"),
            compat_prefix: None,
        },
        // Нет в базе вовсе — пропускается.
        SteamApp {
            appid: 999,
            name: "Unknown Game".into(),
            installdir: "Unknown".into(),
            library: PathBuf::from("/lib"),
            compat_prefix: None,
        },
    ];

    let matches = match_manifests(&db, &apps);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].game_key, "elden-ring");
    assert_eq!(matches[0].matched_by, MatchKind::SteamId);
    assert_eq!(matches[1].game_key, "hades");
    assert_eq!(matches[1].matched_by, MatchKind::NameSlug);
}
#[test]
fn name_fallback_cannot_override_a_different_steam_id() {
    let manifest = parse_manifest_str(MANIFEST).unwrap();
    let apps = vec![SteamApp {
        appid: 999,
        name: "ELDEN RING".into(),
        installdir: "Game".into(),
        library: PathBuf::from("/steam"),
        compat_prefix: None,
    }];
    assert!(match_manifests(&ManifestDb::from_entries(vec![manifest]), &apps).is_empty());
}
