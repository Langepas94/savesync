use super::*;

#[test]
fn rate_limit_error_explains_the_problem() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    for (headers, expected) in [
        (
            "X-RateLimit-Remaining: 0\r\n",
            "GitHub ограничил число запросов",
        ),
        ("", "GitHub Releases вернул HTTP 403"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for line in BufReader::new(&stream).lines() {
                if line.unwrap().is_empty() {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 403 Forbidden\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let error = ureq::get(&format!("http://{address}")).call().unwrap_err();
        assert!(network_error(error).to_string().contains(expected));
        server.join().unwrap();
    }
}

#[test]
fn release_check_distinguishes_an_empty_release_feed_from_denied_access() {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    for repository_status in ["200 OK", "404 Not Found"] {
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
                assert!(!headers
                    .iter()
                    .any(|header| header.to_ascii_lowercase().starts_with("authorization:")));
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
        };
        let result = check_with(&client);
        if repository_status == "200 OK" {
            assert!(matches!(result, Ok(ReleaseCheck::NoPublishedRelease)));
        } else {
            assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::NotFound));
        }
        server.join().unwrap();
    }
}

fn metadata(tag: &str) -> serde_json::Value {
    serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false,"html_url":format!("https://github.com/Langepas94/savesync/releases/tag/{tag}"),"assets":[{"id":1,"name":INSTALLER,"size":100},{"id":2,"name":format!("{INSTALLER}.sha256"),"size":87},{"id":3,"name":format!("{INSTALLER}.sha256.sig"),"size":64}]})
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
fn windows_project_signature_binds_the_release_version() {
    use ed25519_dalek::Signer;
    let key = ed25519_dalek::SigningKey::from_bytes(&[8; 32]);
    let checksum = b"installer checksum";
    let version = semver::Version::new(1, 0, 9);
    let message = signed_checksum_message(checksum, INSTALLER, &version);
    assert_eq!(
        message.as_ref(),
        b"SaveSync Windows update 1.0.9\ninstaller checksum"
    );
    let signature = key.sign(&message).to_bytes();
    assert!(verify_checksum_signature(&key.verifying_key(), &message, &signature).is_ok());
    let relabeled = signed_checksum_message(checksum, INSTALLER, &semver::Version::new(1, 0, 10));
    assert!(verify_checksum_signature(&key.verifying_key(), &relabeled, &signature).is_err());
}

#[test]
fn windows_update_download_requires_valid_signature_and_matching_installer() {
    use ed25519_dalek::Signer;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    let key = ed25519_dalek::SigningKey::from_bytes(&[5; 32]);
    let body = b"installer fixture";
    let version = semver::Version::new(1, 0, 9);
    let checksum = format!("{:x}  {INSTALLER}\n", Sha256::digest(body));
    let signature = key.sign(&signed_checksum_message(
        checksum.as_bytes(),
        INSTALLER,
        &version,
    ));
    let mut release_metadata = metadata("v1.0.9");
    release_metadata["assets"][0]["size"] = body.len().into();
    release_metadata["assets"][1]["size"] = checksum.len().into();
    let release = select_for(
        &serde_json::to_vec(&release_metadata).unwrap(),
        "1.0.8",
        INSTALLER,
    )
    .unwrap()
    .unwrap();

    for (signature_ok, installer_ok) in [(true, true), (true, false), (false, true)] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut signature = signature.to_bytes();
        if !signature_ok {
            signature[0] ^= 1;
        }
        let mut responses = vec![(2, checksum.as_bytes().to_vec()), (3, signature.to_vec())];
        if signature_ok {
            let mut installer = body.to_vec();
            if !installer_ok {
                installer[0] ^= 1;
            }
            responses.push((1, installer));
        }
        let server = std::thread::spawn(move || {
            for (id, bytes) in responses {
                let (mut socket, _) = listener.accept().unwrap();
                let request: Vec<_> = BufReader::new(&socket)
                    .lines()
                    .map(Result::unwrap)
                    .take_while(|line| !line.is_empty())
                    .collect();
                assert_eq!(request[0], format!("GET /releases/assets/{id} HTTP/1.1"));
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .unwrap();
                socket.write_all(&bytes).unwrap();
            }
        });
        let client = Client {
            api_base: format!("http://{address}"),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(2))
                .build(),
        };
        let directory = tempfile::tempdir().unwrap();
        let result = download(&client, &release, directory.path(), || {
            Ok(key.verifying_key())
        });
        server.join().unwrap();
        if signature_ok && installer_ok {
            assert_eq!(std::fs::read(result.unwrap()).unwrap(), body);
        } else {
            assert!(result.is_err());
            assert!(!directory.path().join(INSTALLER).exists());
        }
    }
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
