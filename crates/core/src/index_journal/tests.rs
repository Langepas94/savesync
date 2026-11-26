use super::*;
use crate::index::Version;
pub(crate) fn branch(device: &str, parent: Option<&str>) -> Index {
    let mut index = Index::new();
    index.game_mut("game").versions.push(Version {
        id: format!("v1_{device}_aaaaaaaa"),
        parent: parent.map(str::to_string),
        merged_parents: vec![],
        created_at: 1,
        device: device.into(),
        device_name: None,
        hash: format!("hash-{device}"),
    });
    index
}

#[test]
fn older_client_can_publish_version_without_display_name() {
    let legacy = branch("deck", None);
    let mut named = legacy.clone();
    named.game_mut("game").versions[0].device_name = Some("Steam Deck".into());
    let mut current = legacy.clone();
    merge(&mut current, &named).unwrap();
    merge(&mut current, &legacy).unwrap();
    assert_eq!(
        current.game("game").unwrap().versions[0]
            .device_name
            .as_deref(),
        Some("Steam Deck")
    );
    let mut inconsistent = legacy;
    inconsistent.game_mut("game").versions[0].hash = "different".into();
    assert!(merge(&mut current, &inconsistent).is_err());
}
#[test]
fn concurrent_records_keep_all_branches_and_release_owned_locks() {
    let mut a = branch("pc", None);
    a.acquire_lock("game", "pc", 10);
    let b = branch("deck", None);
    let first = encode(&serde_json::to_vec(&a).unwrap(), "pc", 1).unwrap();
    let second = encode(&serde_json::to_vec(&b).unwrap(), "deck", 1).unwrap();
    let state = read(vec![], vec![first.clone(), second.clone()]).unwrap();
    assert_eq!(state.index.game("game").unwrap().heads().len(), 2);
    assert_eq!(state.index.lock_holder("game", 11).unwrap().device, "pc");
    a.release_lock("game", "pc");
    let release = encode(&serde_json::to_vec(&a).unwrap(), "pc", 2).unwrap();
    assert!(read(vec![], vec![first, second, release])
        .unwrap()
        .index
        .locks
        .is_empty());
}
#[test]
fn detects_modified_publication_and_immutable_version_collision() {
    let index = branch("pc", None);
    let mut record = encode(&serde_json::to_vec(&index).unwrap(), "pc", 1).unwrap();
    record.1.push(b' ');
    assert!(read(vec![], vec![record]).is_err());
    let mut changed = index.clone();
    changed.game_mut("game").versions[0].hash = "other".into();
    let a = encode(&serde_json::to_vec(&index).unwrap(), "pc", 1).unwrap();
    let b = encode(&serde_json::to_vec(&changed).unwrap(), "deck", 1).unwrap();
    assert!(read(vec![], vec![a, b]).is_err());
}
