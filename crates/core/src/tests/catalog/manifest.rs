use super::*;

const ELDEN_RING: &str = r#"
schema: 1
name: Elden Ring
slug: elden-ring
aliases: ["ELDEN RING"]
ids:
  steam: ["1245620"]
paths:
  - pattern: "<winAppData>/EldenRing"
    os: windows
    tags: [save]
  - pattern: "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing"
    os: linux
    tags: [save]
portability:
  status: warn
  reason: "Сейв зашифрован ключом от SteamID64: перенос только между устройствами одного аккаунта"
"#;

const STARDEW: &str = r#"
schema: 1
name: Stardew Valley
slug: stardew-valley
ids:
  steam: ["413150"]
  gog: ["1453375253"]
paths:
  - pattern: "<winAppData>/StardewValley/Saves"
    os: windows
  - pattern: "<xdgConfig>/StardewValley/Saves"
    os: linux
"#;

#[test]
fn parses_full_manifest() {
    let m = parse_manifest_str(ELDEN_RING).unwrap();
    assert_eq!(m.schema, 1);
    assert_eq!(m.slug, "elden-ring");
    assert_eq!(m.ids.steam, vec!["1245620"]);
    assert_eq!(m.paths.len(), 2);
    assert_eq!(m.portability.status, PortabilityStatus::Warn);
    assert!(m.portability.reason.is_some());
    assert!(m.validate().is_empty());
}

#[test]
fn os_defaults_to_any_and_tags_default_save_agnostic() {
    let m = parse_manifest_str(
        "schema: 1\nname: X\nslug: x\npaths:\n  - pattern: \"<home>/x/saves\"\n",
    )
    .unwrap();
    assert_eq!(m.paths[0].os, Os::Any);
    assert!(m.paths[0].tags.is_empty());
    assert!(m.validate().is_empty());
}

#[test]
fn paths_filtered_by_platform() {
    let m = parse_manifest_str(ELDEN_RING).unwrap();
    let win: Vec<_> = m.paths_for(Platform::Windows).collect();
    let lin: Vec<_> = m.paths_for(Platform::Linux).collect();
    assert_eq!(win.len(), 1);
    assert_eq!(lin.len(), 1);
    assert!(win[0].pattern.contains("winAppData"));
    assert!(lin[0].pattern.contains("steamCompat"));
}

#[test]
fn validation_catches_bad_entries() {
    let mut m = parse_manifest_str(STARDEW).unwrap();
    assert!(m.validate().is_empty());

    m.slug = "Not A Slug".into();
    assert!(m.validate().iter().any(|e| e.contains("invalid slug")));

    let mut m2 = parse_manifest_str(STARDEW).unwrap();
    m2.portability.status = PortabilityStatus::Blocked;
    assert!(m2.validate().iter().any(|e| e.contains("requires reason")));

    let mut m3 = parse_manifest_str(STARDEW).unwrap();
    m3.schema = 99;
    assert!(m3
        .validate()
        .iter()
        .any(|e| e.contains("unsupported schema")));
}

#[test]
fn load_tree_tolerates_garbage() {
    let dir = std::env::temp_dir().join(format!("savesync-test-manifest-{}", std::process::id()));
    let shard = dir.join("manifests").join("el");
    std::fs::create_dir_all(&shard).unwrap();
    std::fs::write(shard.join("elden-ring.yaml"), ELDEN_RING).unwrap();
    std::fs::write(shard.join("bad.yaml"), "{{{not yaml").unwrap();
    std::fs::write(dir.join("ignored.txt"), "schema: 1\n").unwrap();

    let (db, errors) = load_manifest_tree(&dir);
    assert_eq!(db.len(), 1);
    assert_eq!(errors.len(), 1);
    assert!(db.get("elden-ring").is_some());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn custom_manifest_roundtrip() {
    let m = Manifest::custom(
        "My Indie Game",
        vec![PathRule {
            location: None,
            pattern: "<documents>/My Indie Game/saves".into(),
            os: Os::Any,
            tags: vec!["save".into()],
        }],
    )
    .unwrap();
    assert_eq!(m.slug, "custom:my-indie-game");
    assert!(m.validate().is_empty());

    // YAML round-trip: запись на диск и загрузка обратно.
    let yaml = m.to_yaml().unwrap();
    let parsed = parse_manifest_str(&yaml).unwrap();
    assert_eq!(parsed.slug, m.slug);
    assert_eq!(parsed.paths.len(), 1);
    assert!(parsed.validate().is_empty());
}

#[test]
fn custom_manifest_rejects_empty_input() {
    assert!(Manifest::custom("", vec![]).is_err());
    assert!(Manifest::custom(
        "X",
        vec![PathRule {
            location: None,
            pattern: "".into(),
            os: Os::Any,
            tags: vec![],
        }],
    )
    .is_err());
}

#[test]
fn new_custom_rule_scans_literal_paths_without_overwriting_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let saves = dir.path().join("Save [Profile]");
    std::fs::create_dir(&saves).unwrap();
    let save = saves.join("slot.sav");
    std::fs::write(&save, b"save").unwrap();
    let manifest = Manifest::custom(
        "My Game",
        vec![PathRule {
            location: None,
            pattern: crate::platform::paths::literal_save_pattern(&saves),
            os: Os::Any,
            tags: vec!["save".into()],
        }],
    )
    .unwrap();
    let custom = dir.path().join("custom");
    manifest.save_new_custom(&custom).unwrap();
    let mut duplicate = manifest.clone();
    duplicate.paths[0].pattern = "<documents>/different".into();
    assert_eq!(
        duplicate.save_new_custom(&custom).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    let (db, errors) = load_manifest_tree(&custom);
    assert!(errors.is_empty());
    let games = crate::catalog::scanner::scan(&db, Platform::current(), &Default::default());
    assert_eq!(games.len(), 1);
    assert_eq!(
        games[0].locations[0].hits,
        [crate::platform::paths::canonical_path(&save)]
    );
}
