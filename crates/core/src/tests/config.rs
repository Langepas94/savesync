use super::*;

#[test]
fn init_persists_device_id() {
    let dir = std::env::temp_dir().join(format!("savesync-test-config-{}", std::process::id()));
    let path = dir.join("config.json");

    let cfg = Config::load_or_init(&path).unwrap();
    assert_eq!(cfg.version, 1);
    assert!(!cfg.device_id.is_empty());
    assert_eq!(cfg.backups.auto_interval_hours, Some(24));
    assert_eq!(
        cfg.backups.dir.as_deref(),
        Some(dir.join("backups").as_path())
    );
    assert!(dir.join("backups").is_dir());

    // Повторная загрузка — тот же device_id (стабильность).
    let cfg2 = Config::load_or_init(&path).unwrap();
    assert_eq!(cfg, cfg2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn device_id_is_slug_prefixed() {
    let id = generate_device_id();
    let (host, suffix) = id.rsplit_once('-').unwrap();
    assert_eq!(suffix.len(), 8);
    assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(crate::catalog::game_key::is_valid_slug(host) || host == "device");
}

#[test]
fn backends_are_optional_and_independent() {
    let json = r#"{"version":1,"device_id":"pc-abcd1234","device_name":"PC","manifests_dir":null}"#;
    let cfg: Config = serde_json::from_str(json).unwrap();
    assert!(cfg.backends.fs_path.is_none());
}
