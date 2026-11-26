use super::*;

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
    let mut app = Application::new();
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
    let mut app = Application::new();
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
