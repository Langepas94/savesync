use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "savesync-test-snap-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Fixture(dir)
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn make_game_dir(fx: &Fixture) -> SaveLocation {
    std::fs::create_dir_all(fx.path("game/saves/nested")).unwrap();
    std::fs::write(fx.path("game/saves/slot1.sav"), b"save-one").unwrap();
    std::fs::write(fx.path("game/saves/nested/slot2.sav"), b"save-two").unwrap();
    let resolved = fx.path("game/saves").to_string_lossy().replace('\\', "/");
    SaveLocation {
        location: None,
        pattern: "<home>/game/saves".into(),
        resolved: format!("{resolved}/**"),
        hits: vec![fx.path("game/saves")],
    }
}

#[test]
fn base_strips_glob() {
    assert_eq!(
        location_base("C:/Users/g/AppData/Roaming/Game/saves/*.sav"),
        PathBuf::from("C:/Users/g/AppData/Roaming/Game/saves/")
    );
    assert_eq!(
        location_base("/home/deck/.config/game"),
        PathBuf::from("/home/deck/.config/")
    );
}

#[test]
fn create_read_verify_roundtrip() {
    let fx = Fixture::new("roundtrip");
    let loc = make_game_dir(&fx);
    let archive = fx.path("snap.zip");

    let manifest = create("test-game", "dev1", &[loc], &archive).unwrap();
    assert_eq!(manifest.game_key, "test-game");
    assert_eq!(manifest.locations.len(), 1);
    assert_eq!(manifest.locations[0].files.len(), 2);
    assert!(!manifest.content_hash.is_empty());

    let loaded = read_manifest(&archive).unwrap();
    assert_eq!(loaded, manifest);
    assert!(verify(&archive).unwrap());
}

#[test]
fn content_hash_changes_with_content() {
    let fx = Fixture::new("hash");
    let loc = make_game_dir(&fx);
    let m1 = create("g", "d", std::slice::from_ref(&loc), &fx.path("1.zip")).unwrap();
    std::fs::write(fx.path("game/saves/slot1.sav"), b"changed").unwrap();
    let m2 = create("g", "d", std::slice::from_ref(&loc), &fx.path("2.zip")).unwrap();
    assert_ne!(m1.content_hash, m2.content_hash);

    // Пересоздание без изменений — тот же хеш (новая версия не нужна).
    let m3 = create("g", "d", std::slice::from_ref(&loc), &fx.path("3.zip")).unwrap();
    assert_eq!(m2.content_hash, m3.content_hash);
}

#[test]
fn extract_restores_files() {
    let fx = Fixture::new("extract");
    let loc = make_game_dir(&fx);
    let archive = fx.path("snap.zip");
    let manifest = create("g", "d", &[loc], &archive).unwrap();

    let dest = fx.path("restored");
    extract_location(&archive, &manifest.locations[0], &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("slot1.sav")).unwrap(), b"save-one");
    assert_eq!(
        std::fs::read(dest.join("nested/slot2.sav")).unwrap(),
        b"save-two"
    );
}
fn write_test_archive(path: &Path, manifest: &SnapshotManifest) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for loc in &manifest.locations {
        for file in &loc.files {
            zip.start_file(format!("{}/{}", loc.archive_dir, file.rel_path), options)
                .unwrap();
            zip.write_all(b"save").unwrap();
        }
    }
    zip.start_file(MANIFEST_NAME, options).unwrap();
    zip.write_all(&serde_json::to_vec(manifest).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

#[test]
fn unsafe_paths_rejected_before_any_write() {
    let fx = Fixture::new("unsafe-paths");
    for path in [
        "../victim",
        "NUL",
        "con.sav",
        "dir/COM1.txt",
        "dir/a?b",
        "dir/a\u{0001}b",
        "/tmp/victim",
        "C:/victim",
        "C:stream",
        "\\\\host\\share",
        "safe/../../victim",
        "safe\\..\\victim",
    ] {
        let loc = LocationManifest {
            location: None,
            pattern: "<home>/saves".into(),
            archive_dir: "loc0".into(),
            files: vec![FileManifest {
                rel_path: path.into(),
                size: 4,
                hash: hash::hash_bytes(b"save").to_hex().to_string(),
            }],
        };
        let m = SnapshotManifest {
            game_key: "game".into(),
            device_id: "d".into(),
            created_at: 1,
            content_hash: "wrong".into(),
            locations: vec![loc.clone()],
        };
        let archive = fx.path("unsafe.zip");
        write_test_archive(&archive, &m);
        assert!(verify(&archive).is_err(), "{path}");
        assert!(
            extract_location(&archive, &loc, &fx.path("dest")).is_err(),
            "{path}"
        );
        assert!(!fx.path("dest").exists());
    }
}

#[test]
fn verify_rejects_tampered_aggregate_and_size() {
    let fx = Fixture::new("aggregate");
    let archive = fx.path("snap.zip");
    let loc = make_game_dir(&fx);
    let mut m = create("game", "d", &[loc], &archive).unwrap();
    m.locations[0].files.truncate(1);
    m.locations[0].files[0].size = 4;
    m.locations[0].files[0].hash = hash::hash_bytes(b"save").to_hex().to_string();
    write_test_archive(&archive, &m);
    assert!(!verify(&archive).unwrap());
    m.content_hash = hash::hash_snapshot(vec![(
        format!("loc0/{}", m.locations[0].files[0].rel_path),
        hash::hash_bytes(b"save"),
    )])
    .to_hex()
    .to_string();
    write_test_archive(&archive, &m);
    assert!(verify(&archive).unwrap());
    m.locations[0].files[0].size = 9;
    write_test_archive(&archive, &m);
    assert!(!verify(&archive).unwrap());
}

#[cfg(unix)]
#[test]
fn extraction_rejects_symlink_destination() {
    let fx = Fixture::new("symlink-destination");
    let archive = fx.path("snap.zip");
    let manifest = create("game", "d", &[make_game_dir(&fx)], &archive).unwrap();
    std::fs::create_dir_all(fx.path("outside")).unwrap();
    std::os::unix::fs::symlink(fx.path("outside"), fx.path("dest")).unwrap();
    assert!(extract_location(&archive, &manifest.locations[0], &fx.path("dest")).is_err());
    assert!(std::fs::read_dir(fx.path("outside"))
        .unwrap()
        .next()
        .is_none());
}

#[test]
fn verify_rejects_case_collisions() {
    let fx = Fixture::new("case-collision");
    let archive = fx.path("snap.zip");
    let mut manifest = create("game", "d", &[make_game_dir(&fx)], &archive).unwrap();
    let mut file = manifest.locations[0].files[0].clone();
    file.rel_path = file.rel_path.to_uppercase();
    manifest.locations[0].files.push(file);
    write_test_archive(&archive, &manifest);
    assert!(verify(&archive).is_err());
}
