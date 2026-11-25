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
        hidden_games: vec![],
        busy: false,
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

#[test]
fn oauth_poll_updates_form_without_opening_browser() {
    use savesync_core::{ipc::OAuthStatusPayload, oauth::Provider};
    struct Fake;
    impl DaemonClient for Fake {
        fn request(&self, method: Method) -> Result<ResponsePayload, String> {
            assert_eq!(
                method,
                Method::OAuthPoll {
                    provider: Provider::Google
                }
            );
            Ok(ResponsePayload::OAuthStatus(OAuthStatusPayload {
                provider: Provider::Google,
                state: "connected".into(),
                message: None,
            }))
        }
    }
    let mut app = SaveSyncApp::new();
    app.client = Arc::new(Fake);
    app.oauth_pending = Some(Provider::Google);
    app.poll_oauth();
    let (provider, result) = app
        .oauth_poll
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    app.apply_oauth_poll(provider, result);
    assert!(app.oauth_pending.is_none());
    assert!(app.cfg.is_none());
    assert!(!app.form_loaded);
}

#[test]
fn oauth_poll_never_waits_for_daemon_on_ui_thread() {
    use savesync_core::{ipc::OAuthStatusPayload, oauth::Provider};
    struct Blocked {
        release: std::sync::Mutex<mpsc::Receiver<()>>,
        entered: mpsc::Sender<()>,
    }
    impl DaemonClient for Blocked {
        fn request(&self, _: Method) -> Result<ResponsePayload, String> {
            let _ = self.entered.send(());
            let _ = self
                .release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2));
            Ok(ResponsePayload::OAuthStatus(OAuthStatusPayload {
                provider: Provider::Google,
                state: "pending".into(),
                message: None,
            }))
        }
    }
    let (release, wait) = mpsc::channel();
    let (entered, started) = mpsc::channel();
    let mut app = SaveSyncApp::new();
    app.client = Arc::new(Blocked {
        release: std::sync::Mutex::new(wait),
        entered,
    });
    app.oauth_pending = Some(Provider::Google);
    let before = Instant::now();
    app.poll_oauth();
    assert!(before.elapsed() < Duration::from_millis(500));
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    release.send(()).unwrap();
    let (provider, result) = app
        .oauth_poll
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    app.apply_oauth_poll(provider, result);
    assert_eq!(app.oauth_pending, Some(Provider::Google));
}

#[test]
fn late_history_cannot_replace_selected_game_history() {
    let mut app = SaveSyncApp::new();
    app.selected = Some("second".into());
    let history = |key: &str| HistoryPayload {
        game_key: key.into(),
        versions: vec![],
        applied: None,
        heads: vec![],
    };
    app.accept_history(history("second"));
    app.accept_history(history("first"));
    assert_eq!(app.history.as_ref().unwrap().game_key, "second");
    app.selected = None;
    app.history = None;
    app.accept_history(history("second"));
    assert!(app.history.is_none());
}

#[test]
fn lan_confirmation_refreshes_peer_without_discarding_other_edits() {
    use savesync_core::ipc::ConfigPayload;
    let config = Config {
        version: 1,
        device_id: "test".into(),
        device_name: "Test".into(),
        manifests_dir: None,
        backends: Default::default(),
        backups: Default::default(),
    };
    let mut updated = config.clone();
    updated.backends.lan = Some(LanConfig {
        port: 47660,
        pin: String::new(),
        peers: vec!["192.168.1.10:47660".into()],
    });
    struct Fake(Config);
    impl DaemonClient for Fake {
        fn request(&self, method: Method) -> Result<ResponsePayload, String> {
            match method {
                Method::Lan { .. } => Ok(ResponsePayload::Lan(Default::default())),
                Method::GetConfig => Ok(ResponsePayload::Config(ConfigPayload {
                    config: self.0.clone(),
                })),
                _ => panic!("unexpected request"),
            }
        }
    }
    let mut app = SaveSyncApp::new();
    app.accept_config(config, false);
    app.form.fs_path = "D:/unsaved-edit".into();
    app.client = Arc::new(Fake(updated.clone()));
    app.lan_action(savesync_core::lan::LanAction::Confirm {
        peer_id: "peer".into(),
    });
    let done = app
        .job
        .take()
        .unwrap()
        .rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert!(done.msg.is_ok());
    app.accept_config(done.config.unwrap(), true);
    assert_eq!(app.form.lan_peers, "192.168.1.10:47660");
    assert_eq!(app.form.fs_path, "D:/unsaved-edit");
    let mut saved = app.cfg.clone().unwrap();
    app.form.apply_to(&mut saved).unwrap();
    assert_eq!(saved.backends.lan, updated.backends.lan);
}

#[test]
fn applied_conflict_branch_can_be_selected_in_game_view() {
    use savesync_core::ipc::VersionEntry;
    struct Fake(mpsc::Sender<Method>);
    impl DaemonClient for Fake {
        fn request(&self, method: Method) -> Result<ResponsePayload, String> {
            self.0.send(method).unwrap();
            Err("test: no real daemon".into())
        }
    }
    let (sent, received) = mpsc::channel();
    let mut app = SaveSyncApp::new();
    app.client = Arc::new(Fake(sent));
    app.selected = Some("test-game".into());
    app.history = Some(HistoryPayload {
        game_key: "test-game".into(),
        applied: Some("local-version".into()),
        heads: vec!["local-version".into(), "remote-version".into()],
        versions: vec![VersionEntry {
            id: "local-version".into(),
            parent: None,
            created_at: 0,
            device: "PC".into(),
            hash: "hash".into(),
        }],
    });
    let context = egui::Context::default();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    let output = context.run(input(), |context| {
        egui::CentralPanel::default().show(context, |ui| app.view_detail(ui));
    });
    let position = output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == "Применить" {
                    return Some(text.pos + text.galley.size() * 0.5);
                }
            }
            None
        })
        .expect("restore button must be visible");
    for pressed in [true, false] {
        let mut click = input();
        click.events = vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        let _ = context.run(click, |context| {
            egui::CentralPanel::default().show(context, |ui| app.view_detail(ui));
        });
    }
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        Method::Restore {
            game_key: "test-game".into(),
            version_id: "local-version".into(),
        }
    );
}
