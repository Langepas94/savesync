use super::*;
use savesync_core::ipc::ResponsePayload;

fn test_runtime(dir: &std::path::Path) -> Runtime {
    let config = savesync_core::config::Config {
        version: 1,
        device_id: "test-device".into(),
        device_name: "Test Device".into(),
        manifests_dir: None,
        backends: Default::default(),
        backups: Default::default(),
    };
    Runtime {
        shutdown_requested: false,
        config,
        base_manifests: Default::default(),
        manifests: Default::default(),
        platform: savesync_core::platform::paths::Platform::current(),
        roots: Default::default(),
        state_dir: dir.join("state"),
        last_scan: Vec::new(),
        hidden_games: Default::default(),
        lan_server: None,
        serving: false,
        oauth_jobs: Default::default(),
    }
}

fn unique_name(test: &str) -> String {
    format!("savesync-test-{test}-{}", std::process::id())
}

fn wait_up(name: &str) {
    for _ in 0..100 {
        if probe(name) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("daemon did not start on {name}");
}

#[test]
fn ipc_roundtrip_status_and_scan() {
    let dir = std::env::temp_dir().join(format!("savesync-test-ipc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let name = unique_name("roundtrip");

    let runtime = Arc::new(Mutex::new(test_runtime(&dir)));
    let server_name = name.clone();
    std::thread::spawn(move || serve(runtime, &server_name));
    wait_up(&name);

    let resp = call(&name, Request::new(1, Method::Status)).unwrap();
    match resp.result {
        Some(ResponsePayload::Status(s)) => {
            assert_eq!(s.device_id, "test-device");
            assert_eq!(s.device_name, "Test Device");
        }
        other => panic!("unexpected: {other:?}"),
    }

    let resp = call(&name, Request::new(2, Method::Scan)).unwrap();
    match resp.result {
        Some(ResponsePayload::Scan(s)) => assert_eq!(s.found, 0),
        other => panic!("unexpected: {other:?}"),
    }

    let save = dir.join("initial.sav");
    let replacement = dir.join("replacement.sav");
    std::fs::write(&save, b"initial save").unwrap();
    std::fs::write(&replacement, b"replacement save").unwrap();
    let add = |path: &std::path::Path| Method::AddCustomGame {
        name: "Readded Game".into(),
        paths: vec![path.to_string_lossy().into_owned()],
    };
    let key = "custom:readded-game";
    for (method, count) in [
        (add(&save), 1),
        (
            Method::SetGameHidden {
                game_key: key.into(),
                hidden: true,
            },
            0,
        ),
        (add(&replacement), 1),
    ] {
        let response = call(&name, Request::new(3, method)).unwrap();
        assert!(
            matches!(response.result, Some(ResponsePayload::Scan(scan)) if scan.found == count)
        );
    }
    let response = call(&name, Request::new(4, Method::Status)).unwrap();
    assert!(
        matches!(response.result, Some(ResponsePayload::Status(status))
        if status.hidden_games.is_empty() && status.games[0].game_key == key && status.games[0].state == "no_versions")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn second_daemon_instance_refused() {
    let dir = std::env::temp_dir().join(format!("savesync-test-ipc2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let name = unique_name("second");

    let rt1 = Arc::new(Mutex::new(test_runtime(&dir)));
    let n1 = name.clone();
    std::thread::spawn(move || serve(rt1, &n1));
    wait_up(&name);

    let rt2 = Arc::new(Mutex::new(test_runtime(&dir)));
    let err = serve(rt2, &name).unwrap_err();
    assert!(
        matches!(
            err.kind(),
            std::io::ErrorKind::AddrInUse
                | std::io::ErrorKind::AlreadyExists
                | std::io::ErrorKind::PermissionDenied
        ),
        "unexpected: {err}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
#[test]
fn shutdown_finishes_even_if_caller_disconnects() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }
    assert!(write_response(
        &mut Broken,
        Response::ok(1, savesync_core::ipc::ResponsePayload::Stopped)
    )
    .unwrap());
    assert!(write_response(&mut Broken, Response::err(1, "ordinary response")).is_err());
}
#[test]
fn status_stays_responsive_while_runtime_is_busy() {
    let directory = std::env::temp_dir().join(format!("ss-busy-status-{}", std::process::id()));
    let runtime = Arc::new(Mutex::new(test_runtime(&directory)));
    let cache = Arc::new(RwLock::new(runtime.lock().unwrap().status()));
    let guard = runtime.lock().unwrap();
    let other = runtime.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let task = std::thread::spawn(move || {
        let _ = sender.send(dispatch(Request::new(1, Method::Status), &other, &cache));
    });
    let response = receiver.recv_timeout(std::time::Duration::from_secs(1));
    drop(guard);
    task.join().unwrap();
    assert!(matches!(response.unwrap().result,Some(ResponsePayload::Status(status))if status.busy));
}
