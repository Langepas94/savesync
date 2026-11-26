use super::*;
use crate::platform::steam::kv_binary::Value;

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

/// Синтетическая запись appinfo с UFS как у реальной игры.
fn entry(appid: u32, name: &str, savefiles: Value) -> AppInfoEntry {
    let common = map(vec![("name", s(name)), ("type", s("Game"))]);
    let ufs = map(vec![("savefiles", savefiles)]);
    AppInfoEntry {
        appid,
        change_number: 0,
        last_updated: 0,
        tree: vec![(
            "appinfo".into(),
            map(vec![("common", common), ("ufs", ufs)]),
        )],
    }
}

#[test]
fn extracts_savefiles_new_and_old_pattern_format() {
    let sf0 = map(vec![
        ("root", s("WinAppDataRoaming")),
        ("path", s("EldenRing")),
        ("patterns", map(vec![("0", s("*.sl2"))])),
        ("recursive", s("1")),
    ]);
    let sf1 = map(vec![
        ("root", s("LinuxXdgDataHome")),
        ("path", s("game\\saves")),
        ("pattern", s("*.sav")),
    ]);
    let savefiles = map(vec![("0", sf0), ("1", sf1)]);
    let e = entry(1, "Game", savefiles);

    let files = e.ufs_savefiles();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].patterns, vec!["*.sl2"]);
    assert!(files[0].recursive);
    assert_eq!(files[1].path, "game/saves"); // backslashes нормализованы
    assert_eq!(files[1].patterns, vec!["*.sav"]);
}

#[test]
fn no_ufs_means_empty() {
    let common = map(vec![("name", s("Tool")), ("type", s("Tool"))]);
    let e = AppInfoEntry {
        appid: 1,
        change_number: 0,
        last_updated: 0,
        tree: vec![("appinfo".into(), map(vec![("common", common)]))],
    };
    assert_eq!(e.app_type(), Some("Tool"));
    assert!(e.ufs_savefiles().is_empty());
}

#[test]
fn windows_root_generates_proton_rule_too() {
    let sf = UfsSaveFile {
        root: "WinAppDataRoaming".into(),
        path: "EldenRing".into(),
        patterns: vec!["*.sl2".into()],
        recursive: false,
    };
    let rules = to_path_rules(1245620, &sf);
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].pattern, "<winAppData>/EldenRing/*.sl2");
    assert_eq!(rules[0].os, Os::Windows);
    assert_eq!(
        rules[1].pattern,
        "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing/*.sl2"
    );
    assert_eq!(rules[1].os, Os::Linux);
}

#[test]
fn steamid_placeholders_become_globs() {
    let sf = UfsSaveFile {
        root: "WinAppDataRoaming".into(),
        path: "Game/{64BitSteamID}".into(),
        patterns: vec!["*".into()],
        recursive: false,
    };
    let rules = to_path_rules(1, &sf);
    assert!(rules[0].pattern.contains("/*/*"));
}

#[test]
fn unsupported_roots_are_skipped() {
    let sf = UfsSaveFile {
        root: "MacHome".into(),
        path: "saves".into(),
        patterns: vec![],
        recursive: false,
    };
    assert!(to_path_rules(1, &sf).is_empty());
    let sf2 = UfsSaveFile {
        root: "MacAppSupport".into(),
        path: "saves".into(),
        patterns: vec![],
        recursive: false,
    };
    assert!(to_path_rules(1, &sf2).is_empty());
}

#[test]
fn linux_roots_map_to_xdg() {
    let sf = UfsSaveFile {
        root: "LinuxXdgDataHome".into(),
        path: "game".into(),
        patterns: vec!["*.sav".into()],
        recursive: true,
    };
    let rules = to_path_rules(1, &sf);
    assert_eq!(rules.len(), 1); // без Proton-двойника
    assert_eq!(rules[0].pattern, "<xdgData>/game/**/*.sav");
    assert_eq!(rules[0].os, Os::Linux);
}
