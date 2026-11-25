use super::*;

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "savesync-test-backend-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn snapshot_roundtrip_and_listing() {
    let dir = fixture("roundtrip");
    let b = FsBackend::new(&dir);

    b.put_snapshot("elden-ring", "v1_d_aaaaaaaa", b"zip1")
        .unwrap();
    b.put_snapshot("elden-ring", "v2_d_bbbbbbbb", b"zip2")
        .unwrap();
    b.put_snapshot("hades", "v1_d_cccccccc", b"zip3").unwrap();

    assert_eq!(
        b.get_snapshot("elden-ring", "v1_d_aaaaaaaa").unwrap(),
        b"zip1"
    );
    assert_eq!(
        b.list_snapshots("elden-ring").unwrap(),
        vec!["v1_d_aaaaaaaa", "v2_d_bbbbbbbb"]
    );
    assert_eq!(b.list_snapshots("absent").unwrap(), Vec::<String>::new());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn index_roundtrip() {
    let dir = fixture("index");
    let b = FsBackend::new(&dir);
    assert!(b.get_index().unwrap().is_none());

    b.put_index(br#"{"version":1}"#).unwrap();
    assert_eq!(b.get_index().unwrap().unwrap(), br#"{"version":1}"#);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rejects_path_escape_before_touching_files() {
    let dir = fixture("unsafe");
    let backend = FsBackend::new(&dir);
    assert!(backend.put_snapshot("../outside", "v1", b"data").is_err());
    assert!(backend
        .put_snapshot("game", "../../outside", b"data")
        .is_err());
    backend
        .put_snapshot("custom:my-game", "v1", b"data")
        .unwrap();
    assert_eq!(
        backend.get_snapshot("custom:my-game", "v1").unwrap(),
        b"data"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
