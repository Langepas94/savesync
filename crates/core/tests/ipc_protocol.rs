use savesync_core::ipc::{Method, Request, Response, ResponsePayload, SyncPayload, SyncResult};

#[test]
fn request_roundtrip() {
    let req = Request::new(
        7,
        Method::Sync {
            game_key: Some("elden-ring".into()),
        },
    );
    let line = req.encode().unwrap();
    assert!(line.ends_with('\n'));
    let back = Request::decode(&line).unwrap();
    assert_eq!(back.id, 7);
    assert_eq!(back.method, req.method);
}

#[test]
fn response_roundtrip_ok_and_err() {
    let ok = Response::ok(
        1,
        ResponsePayload::Sync(SyncPayload {
            results: vec![SyncResult {
                game_key: "g".into(),
                outcome: "uploaded".into(),
                detail: None,
            }],
        }),
    );
    let back = Response::decode(&ok.encode().unwrap()).unwrap();
    assert!(back.result.is_some());
    assert!(back.error.is_none());

    let err = Response::err(2, "backend offline");
    let back = Response::decode(&err.encode().unwrap()).unwrap();
    assert_eq!(back.error.unwrap().message, "backend offline");
}

#[test]
fn method_serde_shapes() {
    let status = serde_json::to_string(&Method::Status).unwrap();
    assert_eq!(status, r#"{"method":"status"}"#);
    let sync = serde_json::to_string(&Method::Sync { game_key: None }).unwrap();
    assert!(sync.contains(r#""method":"sync""#));
    assert!(sync.contains(r#""game_key":null"#));
}
