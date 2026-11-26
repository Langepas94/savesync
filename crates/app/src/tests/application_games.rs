use super::*;

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
    let mut app = Application::new();
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
    let mut app = Application::new();
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
    let draw = |app: &mut Application, input| {
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
fn late_history_cannot_replace_selected_game_history() {
    let mut app = Application::new();
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
    let mut app = Application::new();
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
    let render = |app: &mut Application, events: Vec<egui::Event>| {
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
