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
    assert_eq!(fmt_ts(0), "01.01.1970, 00:00 UTC");
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
fn clicking_the_right_edge_of_a_game_row_loads_its_history() {
    struct Fake(mpsc::Sender<Method>);
    impl DaemonClient for Fake {
        fn request(&self, method: Method) -> Result<ResponsePayload, String> {
            self.0.send(method).unwrap();
            Err("test: no real daemon".into())
        }
    }
    let (sender, receiver) = mpsc::channel();
    let mut app = SaveSyncApp::new();
    app.client = Arc::new(Fake(sender));
    app.daemon_ok = true;
    app.status = Some(StatusPayload {
        last_backup: None,
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
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(720.0, 480.0),
        )),
        ..Default::default()
    };
    let draw = |app: &mut SaveSyncApp, input| {
        context.run(input, |context| {
            egui::CentralPanel::default().show(context, |ui| app.view_games(ui));
        })
    };
    let output = draw(&mut app, input());
    let y = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("Test Game") => {
                Some(text.pos.y + text.galley.size().y / 2.0)
            }
            _ => None,
        })
        .expect("game row must be visible");
    let pos = egui::pos2(675.0, y);
    for pressed in [true, false] {
        let mut click = input();
        click.events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ];
        draw(&mut app, click);
    }
    assert_eq!(app.selected.as_deref(), Some("test-game"));
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        Method::History {
            game_key: "test-game".into()
        }
    );
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
        install_paths: vec![],
        local_files: vec![],
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
    app.form.apply_storage_to(&mut saved);
    assert_eq!(saved.backends.lan, updated.backends.lan);
}

#[test]
fn applied_conflict_branch_can_be_selected_and_restore_action_disappears_afterward() {
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
        install_paths: vec![],
        local_files: vec![],
        applied: Some("local-version".into()),
        heads: vec!["local-version".into(), "remote-version".into()],
        versions: vec![VersionEntry {
            id: "local-version".into(),
            parent: None,
            created_at: 0,
            device: "PC".into(),
            device_name: None,
            hash: "hash".into(),
            channel: None,
            file_count: None,
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
                if text.galley.text() == "Выбрать эту копию" {
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
    assert!(matches!(
        received.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    let render = |app: &mut SaveSyncApp, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, ..input() };
        context.run(raw, |context| {
            egui::CentralPanel::default().show(context, |ui| app.view_detail(ui));
        })
    };
    // A window needs a sizing pass before its controls receive pointer events.
    render(&mut app, vec![]);
    let output = render(&mut app, vec![]);
    let confirm = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Подтвердить восстановление" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("confirmation must be visible");
    for pressed in [true, false] {
        render(
            &mut app,
            vec![
                egui::Event::PointerMoved(confirm),
                egui::Event::PointerButton {
                    pos: confirm,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        Method::Restore {
            game_key: "test-game".into(),
            version_id: "local-version".into(),
        }
    );

    // The daemon records the conflict choice as a new copy with the same content.
    app.job = None;
    let mut resolved = app.history.clone().unwrap();
    let mut current = resolved.versions[0].clone();
    current.id = "resolved-version".into();
    resolved.applied = Some(current.id.clone());
    resolved.heads = vec![current.id.clone()];
    resolved.versions.push(current);
    app.accept_history(resolved);
    let output = context.run(input(), |context| {
        egui::CentralPanel::default().show(context, |ui| app.view_detail(ui));
    });
    let texts: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"На этом устройстве"));
    assert!(!texts.contains(&"Прогресс изменился на нескольких устройствах"));
    assert!(!texts.contains(&"Выбрать эту копию"));
    assert!(!texts.contains(&"Восстановить"));
}

#[test]
fn saving_lan_settings_preserves_other_edits_and_discovery_uses_saved_settings() {
    use savesync_core::{
        ipc::ConfigPayload,
        lan::{LanAction, LanPayload},
    };
    struct Fake {
        requests: mpsc::Sender<Method>,
        fail_save: bool,
    }
    impl DaemonClient for Fake {
        fn request(&self, method: Method) -> Result<ResponsePayload, String> {
            self.requests.send(method.clone()).unwrap();
            match method {
                Method::SaveConfig { config } if !self.fail_save => {
                    Ok(ResponsePayload::Config(ConfigPayload { config: *config }))
                }
                Method::SaveConfig { .. } => Err("port occupied".into()),
                Method::Lan { .. } => Ok(ResponsePayload::Lan(LanPayload {
                    port: Some(47660),
                    ..Default::default()
                })),
                _ => panic!("unexpected request"),
            }
        }
    }
    for fail_save in [false, true] {
        let config = Config {
            version: 1,
            device_id: "test".into(),
            device_name: "Test".into(),
            manifests_dir: None,
            backends: Default::default(),
            backups: Default::default(),
        };
        let (sent, received) = mpsc::channel();
        let mut app = SaveSyncApp::new();
        app.accept_config(config.clone(), false);
        app.form.fs_path = "D:/unsaved-edit".into();
        app.form.backups_keep = "invalid unsaved value".into();
        app.client = Arc::new(Fake {
            requests: sent,
            fail_save,
        });
        app.apply_lan_settings(true);
        let done = app
            .job
            .take()
            .unwrap()
            .rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let Method::SaveConfig { config: saved } = received.recv().unwrap() else {
            panic!("LAN must be saved first");
        };
        assert_eq!(saved.backends.lan.as_ref().unwrap().port, 47660);
        assert_eq!(saved.backends.fs_path, config.backends.fs_path);
        assert_eq!(saved.backups, config.backups);
        if fail_save {
            assert_eq!(done.msg.unwrap_err(), "port occupied");
            assert!(received.try_recv().is_err());
            continue;
        }
        assert_eq!(
            received.recv().unwrap(),
            Method::Lan {
                action: LanAction::Status
            }
        );
        assert!(received.try_recv().is_err());
        assert!(done.msg.is_ok());
        app.accept_config(done.config.unwrap(), done.lan_config_only);
        assert!(app.cfg.as_ref().unwrap().backends.lan.is_some());
        assert_eq!(app.form.fs_path, "D:/unsaved-edit");
        assert_eq!(app.form.backups_keep, "invalid unsaved value");
        app.form.lan_port = "invalid unsaved port".into();
        app.lan_action(LanAction::Discover);
        let done = app
            .job
            .take()
            .unwrap()
            .rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(done.msg.is_ok());
        assert_eq!(
            received.try_recv().unwrap(),
            Method::Lan {
                action: LanAction::Discover
            }
        );
        assert!(received.try_recv().is_err());
    }
}

#[test]
fn lan_rejects_invalid_pending_port_before_rpc() {
    let mut app = SaveSyncApp::new();
    app.accept_config(
        Config {
            version: 1,
            device_id: "test".into(),
            device_name: "Test".into(),
            manifests_dir: None,
            backends: Default::default(),
            backups: Default::default(),
        },
        false,
    );
    app.form.lan_port = "invalid".into();
    app.apply_lan_settings(true);
    assert!(app.job.is_none());
    assert!(app.notice.as_ref().unwrap().1);
}

#[test]
fn lan_status_displays_trusted_device_without_discarding_search_results() {
    let mut app = SaveSyncApp::new();
    let config = Config {
        version: 1,
        device_id: "pc".into(),
        device_name: "PC".into(),
        manifests_dir: None,
        backends: Default::default(),
        backups: Default::default(),
    };
    app.accept_config(config, false);
    app.lan_discovered = Some(vec![DiscoveredPeer {
        identity: "deck".into(),
        name: "Steam Deck".into(),
        address: "192.168.1.2:47660".into(),
    }]);
    let (sender, receiver) = mpsc::channel();
    app.lan_poll = Some(receiver);
    sender
        .send(Ok(ResponsePayload::Lan(LanPayload {
            peers: vec![savesync_core::lan::PeerStatus {
                peer: savesync_core::lan::Peer {
                    id: "deck".into(),
                    name: Some("Steam Deck".into()),
                    address: None,
                    public_key: Vec::new(),
                    certificate: Vec::new(),
                },
                confirmed: true,
                sas: None,
            }],
            ..Default::default()
        })))
        .unwrap();
    app.collect_lan_poll();

    assert_eq!(app.lan_discovered.as_ref().unwrap().len(), 1);
    let context = egui::Context::default();
    let output = context.run(egui::RawInput::default(), |context| {
        egui::CentralPanel::default().show(context, |ui| app.view_connections(ui));
    });
    assert!(output.shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::Text(text) => text.galley.text() == "Steam Deck",
        _ => false,
    }));
}
