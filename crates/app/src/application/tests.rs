use super::*;
use savesync_core::config::{LanConfig, YandexConfig};
use savesync_core::ipc::SyncResult;

#[test]
fn civil_from_days_matches_known_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    // 2026-01-01 = 20454 (2025 не високосный), +328 дней до 25 ноября.
    assert_eq!(civil_from_days(20_782), (2026, 11, 25));
}

#[test]
fn fmt_ts_zero_is_epoch() {
    assert_eq!(fmt_ts(0), "1970-01-01 00:00");
}

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
    form.apply_to(&mut back).unwrap();
    assert_eq!(back, cfg);
}

#[test]
fn form_rejects_bad_port() {
    let form = ConfigForm {
        lan_enabled: true,
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
    assert!(form.apply_to(&mut cfg).is_err());
}

#[test]
fn sync_summary_formats_outcomes() {
    let p = ResponsePayload::Sync(savesync_core::ipc::SyncPayload {
        results: vec![SyncResult {
            game_key: "elden-ring".into(),
            outcome: "uploaded".into(),
            detail: Some("[fs] uploaded".into()),
        }],
    });
    let s = sync_summary(&p);
    assert!(s.contains("elden-ring"));
    assert!(s.contains("uploaded"));
}

#[test]
fn refresh_runs_scan_through_injected_client_without_a_desktop() {
    use savesync_core::ipc::ScanPayload;
    use std::sync::Mutex;
    struct FakeClient(Arc<Mutex<Vec<Method>>>);
    impl DaemonClient for FakeClient {
        fn request(&self, method: Method) -> Result<ResponsePayload, String> {
            self.0.lock().unwrap().push(method);
            Ok(ResponsePayload::Scan(ScanPayload {
                found: 0,
                games: vec![],
            }))
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut app = SaveSyncApp::new();
    app.client = Arc::new(FakeClient(calls.clone()));
    app.refresh_games();
    let done = app
        .job
        .take()
        .unwrap()
        .rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert!(done.msg.unwrap().contains('0'));
    assert!(done.refresh_status);
    assert_eq!(*calls.lock().unwrap(), vec![Method::Scan]);
}

#[test]
fn game_view_renders_with_an_egui_context_without_a_window() {
    let mut app = SaveSyncApp::new();
    app.daemon_ok = true;
    app.status = Some(StatusPayload {
        daemon_version: "test".into(),
        device_id: "qa".into(),
        device_name: "QA".into(),
        games: vec![savesync_core::ipc::GameStatus {
            game_key: "test-game".into(),
            display_name: "Test Game".into(),
            versions: 2,
            state: "conflict".into(),
        }],
    });
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| app.view_games(ui));
    });
    assert!(!output.shapes.is_empty());
    assert!(app.job.is_none());
}
