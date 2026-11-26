use super::*;

#[test]
fn form_roundtrip_preserves_tokens() {
    let cfg = Config {
        version: 1,
        device_id: "pc-12345678".into(),
        device_name: "PC".into(),
        manifests_dir: None,
        backends: savesync_core::config::BackendsConfig {
            fs_path: Some(std::path::PathBuf::from("D:/saves")),
            yandex: Some(YandexConfig {
                token: "y0_tok".into(),
            }),
            google: None,
            lan: Some(LanConfig {
                port: 47660,
                pin: "1234".into(),
                peers: vec!["deck:47660".into()],
            }),
        },
        backups: savesync_core::config::BackupsSettings {
            dir: Some(std::path::PathBuf::from("D:/backups")),
            keep: Some(5),
            auto_interval_hours: Some(24),
        },
    };
    let form = ConfigForm::from_config(&cfg);
    let mut back = cfg.clone();
    form.apply_storage_to(&mut back);
    form.apply_backups_to(&mut back).unwrap();
    form.apply_lan_to(&mut back, true).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn form_rejects_bad_port() {
    let form = ConfigForm {
        lan_port: "abc".into(),
        ..Default::default()
    };
    let mut cfg = Config {
        version: 1,
        device_id: "d".into(),
        device_name: "d".into(),
        manifests_dir: None,
        backends: Default::default(),
        backups: Default::default(),
    };
    assert!(form.apply_lan_to(&mut cfg, true).is_err());
}
