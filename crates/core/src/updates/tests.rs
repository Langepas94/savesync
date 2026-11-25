use super::*;

#[test]
fn release_check_distinguishes_an_empty_release_feed_from_denied_access() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    for (repository_status, token) in [("200 OK", Some("test")), ("404 Not Found", None)] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api_base = format!("http://{}/repos/savesync", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, status) in [
                ("/repos/savesync/releases/latest", "404 Not Found"),
                ("/repos/savesync", repository_status),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let headers: Vec<_> = BufReader::new(&stream)
                    .lines()
                    .map(Result::unwrap)
                    .take_while(|line| !line.is_empty())
                    .collect();
                assert_eq!(headers[0], format!("GET {path} HTTP/1.1"));
                assert_eq!(
                    headers
                        .iter()
                        .any(|header| header.eq_ignore_ascii_case("authorization: Bearer test")),
                    token.is_some()
                );
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let client = Client {
            api_base,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(2))
                .build(),
            token: token.map(str::to_owned),
        };
        let result = check_with(&client);
        if token.is_some() {
            assert!(matches!(result, Ok(ReleaseCheck::NoPublishedRelease)));
        } else {
            assert!(
                matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
            );
        }
        server.join().unwrap();
    }
}

fn metadata(tag: &str) -> serde_json::Value {
    serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false,"html_url":format!("https://github.com/Langepas94/savesync/releases/tag/{tag}"),"assets":[{"id":1,"name":INSTALLER,"size":100},{"id":2,"name":format!("{INSTALLER}.sha256"),"size":87}]})
}
#[test]
fn stable_semver_check_rejects_old_and_prerelease() {
    assert!(select_for(
        &serde_json::to_vec(&metadata("v1.2.0")).unwrap(),
        "1.1.0",
        INSTALLER
    )
    .unwrap()
    .is_some());
    assert!(select_for(
        &serde_json::to_vec(&metadata("v1.0.0")).unwrap(),
        "1.1.0",
        INSTALLER
    )
    .unwrap()
    .is_none());
    assert!(select_for(
        &serde_json::to_vec(&metadata("v2.0.0-beta.1")).unwrap(),
        "1.1.0",
        INSTALLER
    )
    .unwrap()
    .is_none());
}
#[test]
fn ambiguous_or_unsafe_release_assets_are_rejected() {
    let mut value = metadata("v2.0.0");
    let duplicate = value["assets"][0].clone();
    value["assets"].as_array_mut().unwrap().push(duplicate);
    assert!(select_for(&serde_json::to_vec(&value).unwrap(), "1.0.0", INSTALLER).is_err());
    let mut value = metadata("v2.0.0");
    value["html_url"] = "https://example.com/install.exe".into();
    assert!(select_for(&serde_json::to_vec(&value).unwrap(), "1.0.0", INSTALLER).is_err());
}
#[test]
fn steamos_requires_its_own_signed_package() {
    let mut value = metadata("v2.0.0");
    value["assets"] = serde_json::json!([
        {"id": 1, "name": "savesync-steamos.tar.gz", "size": 100},
        {"id": 2, "name": "savesync-steamos.tar.gz.sha256", "size": 100},
        {"id": 3, "name": "savesync-steamos.tar.gz.sha256.sig", "size": 64}
    ]);
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(select_for(&bytes, "1.0.0", "savesync-steamos.tar.gz")
        .unwrap()
        .is_some());
    value["assets"].as_array_mut().unwrap().pop();
    assert!(select_for(
        &serde_json::to_vec(&value).unwrap(),
        "1.0.0",
        "savesync-steamos.tar.gz"
    )
    .is_err());
}
#[test]
fn signed_checksum_rejects_tampering_and_wrong_key() {
    use ed25519_dalek::Signer;
    let key = ed25519_dalek::SigningKey::from_bytes(&[3; 32]);
    let bytes = b"signed checksum";
    let signature = key.sign(bytes).to_bytes();
    assert!(verify_checksum_signature(&key.verifying_key(), bytes, &signature).is_ok());
    assert!(verify_checksum_signature(&key.verifying_key(), b"tampered", &signature).is_err());
    let other = ed25519_dalek::SigningKey::from_bytes(&[4; 32]);
    assert!(verify_checksum_signature(&other.verifying_key(), bytes, &signature).is_err());
}
#[test]
fn checksum_is_bound_to_expected_installer_name() {
    let hash = "a".repeat(64);
    assert_eq!(
        checksum(format!("{hash}  {INSTALLER}\n").as_bytes()).unwrap(),
        hash
    );
    assert!(checksum(format!("{hash} other.exe").as_bytes()).is_err());
    assert!(checksum(b"bad checksum").is_err());
}
#[cfg(windows)]
#[test]
fn unsigned_file_is_never_accepted_as_installer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("savesync-setup.exe");
    std::fs::write(&path, b"unsigned data").unwrap();
    assert!(verify_installer(&path, &semver::Version::new(1, 0, 0)).is_err());
}
