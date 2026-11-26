use super::*;

#[test]
fn bytes_hash_is_stable() {
    assert_eq!(hash_bytes(b"save-data"), hash_bytes(b"save-data"));
    assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
}

#[test]
fn file_hash_matches_bytes() {
    let path = std::env::temp_dir().join(format!("savesync-hash-{}", std::process::id()));
    std::fs::write(&path, b"content").unwrap();
    let h = hash_file(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(h, hash_bytes(b"content"));
}

#[test]
fn snapshot_hash_is_order_independent() {
    let a = ("saves/a.sav".to_string(), hash_bytes(b"1"));
    let b = ("saves/b.sav".to_string(), hash_bytes(b"2"));
    assert_eq!(
        hash_snapshot(vec![a.clone(), b.clone()]),
        hash_snapshot(vec![b, a])
    );
}

#[test]
fn snapshot_hash_changes_with_content() {
    let a = ("save.sav".to_string(), hash_bytes(b"v1"));
    let b = ("save.sav".to_string(), hash_bytes(b"v2"));
    assert_ne!(hash_snapshot(vec![a]), hash_snapshot(vec![b]));
}
