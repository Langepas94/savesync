use super::*;

fn v(id: &str, parent: Option<&str>, hash: &str) -> Version {
    Version {
        id: id.into(),
        parent: parent.map(Into::into),
        merged_parents: Vec::new(),
        created_at: 1,
        device: "dev".into(),
        hash: hash.into(),
    }
}

#[test]
fn linear_history_has_single_head() {
    let mut idx = Index::new();
    idx.add_version("g", v("v1", None, "a"));
    idx.add_version("g", v("v2", Some("v1"), "b"));
    let game = idx.game("g").unwrap();
    assert_eq!(game.head().unwrap().id, "v2");
    assert!(game.is_ancestor("v1", "v2"));
    assert!(!game.is_ancestor("v2", "v1"));
}

#[test]
fn branching_is_detected_as_conflict() {
    let mut idx = Index::new();
    idx.add_version("g", v("v1", None, "a"));
    idx.add_version("g", v("v2-pc", Some("v1"), "b"));
    idx.add_version("g", v("v2-deck", Some("v1"), "c"));
    let heads = idx.game("g").unwrap().heads();
    assert_eq!(heads.len(), 2);
    assert!(idx.game("g").unwrap().head().is_none());
}

#[test]
fn add_version_dedupes_by_hash_and_parent() {
    let mut idx = Index::new();
    assert!(idx.add_version("g", v("v1", None, "a")));
    // Тот же контент от той же базы — не версия (пустые изменения).
    assert!(!idx.add_version("g", v("v1-other-device", None, "a")));
    // Тот же контент от другой базы — новая ветка, сохраняем.
    assert!(idx.add_version("g", v("v2", Some("v1"), "a")));
    assert_eq!(idx.game("g").unwrap().versions.len(), 2);
}

#[test]
fn merge_unions_versions_by_id() {
    let mut a = Index::new();
    a.add_version("g", v("v1", None, "a"));
    let mut b = Index::new();
    b.add_version("g", v("v1", None, "a"));
    b.add_version("g", v("v2", Some("v1"), "b"));

    a.merge(&b);
    assert_eq!(a.game("g").unwrap().versions.len(), 2);
    // Повторный merge идемпотентен.
    a.merge(&b);
    assert_eq!(a.game("g").unwrap().versions.len(), 2);
}

#[test]
fn save_load_roundtrip_and_atomicity() {
    let dir = std::env::temp_dir().join(format!("savesync-test-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("index.json");

    let mut idx = Index::new();
    idx.add_version("elden-ring", v("v1", None, "abc"));
    idx.save(&path).unwrap();

    let loaded = Index::load(&path).unwrap();
    assert_eq!(loaded, idx);
    // Отсутствующий файл — пустой индекс, а не ошибка.
    assert_eq!(Index::load(&dir.join("nope.json")).unwrap(), Index::new());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn version_id_format() {
    let id = version_id(1_700_000_000_000, "deck", "abcdef0123456789");
    assert_eq!(id, "v1700000000000_deck_abcdef01");
}

#[test]
fn lock_acquire_release_and_staleness() {
    let mut idx = Index::new();
    assert!(idx.acquire_lock("g", "pc", 1000));
    // Чужое устройство не перехватывает свежий лок.
    assert!(!idx.acquire_lock("g", "deck", 1001));
    // Своё устройство продлевает heartbeat.
    assert!(idx.acquire_lock("g", "pc", 2000));
    assert_eq!(idx.lock_holder("g", 3000).unwrap().device, "pc");
    // Чужой лок нельзя снять.
    idx.release_lock("g", "deck");
    assert!(idx.lock_holder("g", 3000).is_some());
    // Свой — можно.
    idx.release_lock("g", "pc");
    assert!(idx.lock_holder("g", 3000).is_none());
    // Протухший лок не держит игру.
    assert!(idx.acquire_lock("g", "pc", 10_000));
    assert!(idx.lock_holder("g", 10_000 + LOCK_STALE_MS).is_none());
    assert!(idx.acquire_lock("g", "deck", 10_000 + LOCK_STALE_MS));
}

#[test]
fn merge_keeps_freshest_lock() {
    let mut a = Index::new();
    a.acquire_lock("g", "pc", 1000);
    let mut b = Index::new();
    b.acquire_lock("g", "deck", 2000);

    a.merge(&b);
    assert_eq!(a.locks["g"].device, "deck");
    assert_eq!(a.locks["g"].heartbeat, 2000);
    // Обратный merge не откатывает на более старый.
    b.merge(&a);
    assert_eq!(b.locks["g"].heartbeat, 2000);
}

#[test]
fn locks_survive_save_load() {
    let dir = std::env::temp_dir().join(format!("savesync-test-locks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("index.json");

    let mut idx = Index::new();
    idx.acquire_lock("elden-ring", "pc", 42);
    idx.save(&path).unwrap();
    let loaded = Index::load(&path).unwrap();
    assert_eq!(loaded.locks["elden-ring"].heartbeat, 42);
    // Старый индекс без поля locks читается как пустой набор.
    let legacy = br#"{"version":1,"games":{}}"#;
    let parsed: Index = serde_json::from_slice(legacy).unwrap();
    assert!(parsed.locks.is_empty());

    std::fs::remove_dir_all(&dir).ok();
}
#[test]
fn empty_and_cyclic_histories_do_not_panic_or_hang() {
    assert!(GameIndex::default().head().is_none());
    let mut idx = Index::new();
    idx.add_version("g", v("v1", Some("v2"), "a"));
    idx.add_version("g", v("v2", Some("v1"), "b"));
    let game = idx.game("g").unwrap();
    assert!(game.head().is_none());
    assert!(!game.is_ancestor("missing", "v1"));
}
