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

#[test]
fn stale_writers_merge_under_filesystem_lock() {
    let directory = tempfile::tempdir().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut tasks = Vec::new();
    for number in 0..8 {
        let root = directory.path().to_path_buf();
        let barrier = barrier.clone();
        tasks.push(std::thread::spawn(move || {
            let backend = FsBackend::new(root);
            let writer = format!("writer{number}");
            let index = crate::sync::index_journal::tests::branch(&writer, None);
            barrier.wait();
            backend
                .publish_index(&serde_json::to_vec(&index).unwrap(), &writer)
                .unwrap();
        }));
    }
    for task in tasks {
        task.join().unwrap();
    }
    let backend = FsBackend::new(directory.path());
    let index = crate::sync::index_journal::parse(&backend.get_index().unwrap().unwrap()).unwrap();
    assert_eq!(index.game("game").unwrap().versions.len(), 8);
}

#[test]
fn stale_foreign_lock_does_not_reappear_after_release() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FsBackend::new(directory.path());
    let mut owner = crate::sync::index::Index::new();
    owner.acquire_lock("game", "pc", 1);
    backend
        .publish_index(&serde_json::to_vec(&owner).unwrap(), "pc")
        .unwrap();
    let stale = backend.get_index().unwrap().unwrap();
    owner.release_lock("game", "pc");
    backend
        .publish_index(&serde_json::to_vec(&owner).unwrap(), "pc")
        .unwrap();
    backend.publish_index(&stale, "deck").unwrap();
    assert!(
        crate::sync::index_journal::parse(&backend.get_index().unwrap().unwrap())
            .unwrap()
            .locks
            .is_empty()
    );
}
