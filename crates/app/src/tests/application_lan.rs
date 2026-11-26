use super::*;

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
    let mut app = Application::new();
    app.accept_config(config, false);
    app.form.fs_path = "D:/unsaved-edit".into();
    app.client = Arc::new(Fake(updated.clone()));
    app.lan_action(savesync_core::backends::lan::LanAction::Confirm {
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
        let mut app = Application::new();
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
    let mut app = Application::new();
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
    let mut app = Application::new();
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
            peers: vec![savesync_core::backends::lan::PeerStatus {
                peer: savesync_core::backends::lan::Peer {
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
