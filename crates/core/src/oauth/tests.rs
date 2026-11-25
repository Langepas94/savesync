use super::*;
use crate::credentials::{
    self,
    tests::{record, MemoryStore},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
fn server(status: u16, body: &str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let body = body.to_string();
    let task = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut byte = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
        }
        let header = String::from_utf8(bytes).unwrap();
        let length = header
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|n| n.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        let mut content = vec![0; length];
        socket.read_exact(&mut content).unwrap();
        write!(socket,"HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        String::from_utf8(content).unwrap()
    });
    (endpoint, task)
}
#[test]
fn expired_token_refresh_rotates_and_persists() {
    let store = MemoryStore::default();
    let token = record();
    let reference = credentials::save(&store, None, &token).unwrap();
    let (endpoint, task) = server(
        200,
        r#"{"access_token":"new-access","refresh_token":"rotated-refresh","token_type":"Bearer","expires_in":3600}"#,
    );
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        protocol::access_token_with(
            &reference,
            Provider::Google,
            false,
            &store,
            &endpoint,
            directory.path()
        )
        .unwrap(),
        "new-access"
    );
    let request = task.join().unwrap();
    assert!(request.contains("grant_type=refresh_token"));
    assert!(request.contains("refresh_token=refresh-secret"));
    let updated = credentials::load(&store, &reference, Provider::Google).unwrap();
    assert_eq!(updated.refresh_token.as_deref(), Some("rotated-refresh"));
    assert!(updated.expires_at.unwrap() > now_seconds() + 3500);
    assert_eq!(
        protocol::access_token_with(
            &reference,
            Provider::Google,
            false,
            &store,
            "http://127.0.0.1:1",
            directory.path()
        )
        .unwrap(),
        "new-access"
    );
}
#[test]
fn refresh_without_rotation_preserves_refresh_token() {
    let token = record();
    let (endpoint, task) = server(
        200,
        r#"{"access_token":"new","token_type":"bearer","expires_in":3600}"#,
    );
    let updated = protocol::refresh_at(&token, &endpoint).unwrap();
    task.join().unwrap();
    assert_eq!(updated.refresh_token, token.refresh_token);
}
#[test]
fn refresh_error_is_sanitized_and_keeps_credentials() {
    let store = MemoryStore::default();
    let reference = credentials::save(&store, None, &record()).unwrap();
    let (endpoint, task) = server(
        400,
        r#"{"error":"refresh-secret client-secret invalid_grant"}"#,
    );
    let directory = tempfile::tempdir().unwrap();
    let error = protocol::access_token_with(
        &reference,
        Provider::Google,
        true,
        &store,
        &endpoint,
        directory.path(),
    )
    .unwrap_err()
    .to_string();
    task.join().unwrap();
    assert!(error.contains("HTTP 400"));
    assert!(!error.contains("secret"));
    assert_eq!(
        credentials::load(&store, &reference, Provider::Google)
            .unwrap()
            .access_token,
        "access-secret"
    );
}
#[test]
fn insufficient_scope_is_rejected() {
    let (endpoint, task) = server(
        200,
        r#"{"access_token":"new","token_type":"bearer","scope":"email"}"#,
    );
    assert!(protocol::refresh_at(&record(), &endpoint).is_err());
    task.join().unwrap();
}
fn session() -> AuthorizationSession {
    AuthorizationSession::begin(ClientSettings {
        provider: Provider::Google,
        client_id: "test-client".into(),
        client_secret: None,
        redirect_port: 0,
    })
    .unwrap()
}
#[test]
fn authorization_uses_random_state_and_s256_loopback() {
    let first = session();
    let second = session();
    let url = url::Url::parse(&first.authorization_url).unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["access_type"], "offline");
    assert!(params["redirect_uri"].starts_with("http://127.0.0.1:"));
    assert!(params["state"].len() >= 32);
    assert_ne!(first.authorization_url, second.authorization_url);
    assert!(!first.authorization_url.contains("code_verifier"));
}
#[test]
fn cancelled_and_expired_sessions_return_without_credentials() {
    assert!(session()
        .complete(Arc::new(AtomicBool::new(true)), Duration::from_secs(1))
        .is_err());
    assert!(session()
        .complete(Arc::new(AtomicBool::new(false)), Duration::ZERO)
        .is_err());
}

fn callback(url: &url::Url, query: &str) -> String {
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .write_all(
            format!("GET /oauth/callback?{query} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
        )
        .unwrap();
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(socket);
    let mut response = String::new();
    reader.read_line(&mut response).unwrap();
    let mut length = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("callback response must declare its length")];
    reader.read_exact(&mut body).unwrap();
    response.push_str(&String::from_utf8(body).unwrap());
    response
}
#[test]
fn forged_callback_rejected_then_valid_code_exchanged_with_pkce() {
    let (endpoint, token_task) = server(
        200,
        r#"{"access_token":"access","refresh_token":"refresh","token_type":"Bearer","expires_in":3600}"#,
    );
    let session = session().test_endpoint(endpoint);
    let authorization = url::Url::parse(&session.authorization_url).unwrap();
    let params: std::collections::HashMap<String, String> =
        authorization.query_pairs().into_owned().collect();
    let redirect = url::Url::parse(&params["redirect_uri"]).unwrap();
    let state = params["state"].clone();
    let task = std::thread::spawn(move || {
        session.complete(Arc::new(AtomicBool::new(false)), Duration::from_secs(10))
    });
    assert!(callback(&redirect, "code=forged&state=wrong").contains("400"));
    assert!(callback(
        &redirect,
        &format!("code=valid-code&state={state}&state={state}")
    )
    .contains("400"));
    assert!(callback(&redirect, &format!("code=valid-code&state={state}")).contains("200"));
    assert_eq!(task.join().unwrap().unwrap().access_token, "access");
    let request = token_task.join().unwrap();
    let fields: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(request.as_bytes())
            .into_owned()
            .collect();
    assert_eq!(fields["code"], "valid-code");
    assert_eq!(fields["redirect_uri"], params["redirect_uri"]);
    let verifier = oauth2::PkceCodeVerifier::new(fields["code_verifier"].clone());
    assert_eq!(
        oauth2::PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str(),
        params["code_challenge"]
    );
}
#[test]
fn valid_state_denial_returns_permission_error() {
    let session = session();
    let authorization = url::Url::parse(&session.authorization_url).unwrap();
    let params: std::collections::HashMap<String, String> =
        authorization.query_pairs().into_owned().collect();
    let redirect = url::Url::parse(&params["redirect_uri"]).unwrap();
    let state = params["state"].clone();
    let task = std::thread::spawn(move || {
        session.complete(Arc::new(AtomicBool::new(false)), Duration::from_secs(5))
    });
    assert!(callback(&redirect, &format!("error=access_denied&state={state}")).contains("403"));
    assert_eq!(
        task.join().unwrap().err().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
}
