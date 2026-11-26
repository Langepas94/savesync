use super::*;

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "savesync-test-backup-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_save(home: &Path, content: &str) -> Vec<SaveLocation> {
    let dir = home.join("game/saves");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("slot1.sav"), content).unwrap();
    let resolved = dir.to_string_lossy().replace('\\', "/");
    vec![SaveLocation {
        location: None,
        pattern: "<home>/game/saves".into(),
        resolved: resolved.clone(),
        hits: vec![PathBuf::from(resolved)],
    }]
}

#[test]
fn create_list_and_prune() {
    let dir = fixture("create");
    let home = dir.join("home");
    let dest = dir.join("backups");

    let path = create_backup("test-game", "pc", &write_save(&home, "v1"), &dest).unwrap();
    assert!(path.extension().and_then(|e| e.to_str()) == Some(BACKUP_EXT));
    // Файл самодостаточен: verify + manifest.
    assert!(snapshot::verify(&path).unwrap());

    // Ещё два бэкапа с нарастающим ts.
    std::thread::sleep(std::time::Duration::from_millis(2));
    create_backup("test-game", "pc", &write_save(&home, "v2"), &dest).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    create_backup("test-game", "pc", &write_save(&home, "v3"), &dest).unwrap();

    let all = list_backups(&dest, None).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].game_key, "test-game");
    assert!(all[0].created_at >= all[1].created_at);

    // Фильтр по игре.
    assert!(list_backups(&dest, Some("other-game")).unwrap().is_empty());

    // Ротация: оставить 2 свежих.
    assert_eq!(prune_backup_files(&dest, "test-game", 2).unwrap(), 1);
    assert_eq!(list_backups(&dest, None).unwrap().len(), 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn prune_dir_backups_keeps_newest() {
    let dir = fixture("dirs");
    let backups = dir.join("backups/test-game");
    for ts in ["100", "200", "300", "400"] {
        std::fs::create_dir_all(backups.join(ts)).unwrap();
    }
    assert_eq!(prune_dir_backups(&dir, "test-game", 2).unwrap(), 2);
    let left: Vec<String> = std::fs::read_dir(&backups)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["300", "400"]);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_skips_garbage() {
    let dir = fixture("garbage");
    std::fs::write(dir.join("junk.savesync-backup"), b"not a zip").unwrap();
    std::fs::write(dir.join("note.txt"), b"hi").unwrap();
    assert!(list_backups(&dir, None).unwrap().is_empty());
    // Отсутствующая папка — пустой список.
    assert!(list_backups(&dir.join("nope"), None).unwrap().is_empty());

    std::fs::remove_dir_all(&dir).ok();
}
