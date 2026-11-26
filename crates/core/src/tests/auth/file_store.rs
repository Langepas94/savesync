use super::*;
#[test]
fn encrypted_store_persists_and_wrong_password_cannot_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault.json");
    let store = FileStore::new(path.clone(), "a long testing passphrase".into()).unwrap();
    store.put("record/access", "very-private-token").unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("very-private-token"));
    let reopened = FileStore::new(path.clone(), "a long testing passphrase".into()).unwrap();
    assert_eq!(reopened.get("record/access").unwrap(), "very-private-token");
    let wrong = FileStore::new(path.clone(), "a different testing passphrase".into()).unwrap();
    assert!(wrong.get("record/access").is_err());
    assert!(wrong.put("record/access", "replacement").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    reopened.delete("record/access").unwrap();
    assert_eq!(
        reopened.get("record/access").unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
}
#[test]
fn tampering_is_detected_without_leaking_secrets() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault.json");
    let store = FileStore::new(path.clone(), "a long testing passphrase".into()).unwrap();
    store.put("key", "secret").unwrap();
    let mut envelope: Envelope = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    envelope.ciphertext[0] ^= 1;
    std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    assert!(store.get("key").is_err());
    assert!(!store.get("key").unwrap_err().to_string().contains("secret"));
}
