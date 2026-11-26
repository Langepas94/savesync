use super::*;
#[test]
fn verified_disk_cache_avoids_download_and_repairs_corruption() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = b"immutable publication".to_vec();
    let hash = blake3::hash(&bytes).to_hex();
    let name = format!("{}{hash}.json", crate::sync::index_journal::PREFIX);
    let cache = Cache {
        directory: Some(directory.path().into()),
        ..Cache::default()
    };
    assert_eq!(cache.get(&name, || Ok(bytes.clone())).unwrap(), bytes);
    let reopened = Cache {
        directory: cache.directory.clone(),
        ..Cache::default()
    };
    assert_eq!(
        reopened
            .get(&name, || panic!("must use verified disk cache"))
            .unwrap(),
        bytes
    );
    std::fs::write(directory.path().join(format!("{hash}.json")), b"corrupt").unwrap();
    let reopened = Cache {
        directory: cache.directory.clone(),
        ..Cache::default()
    };
    assert_eq!(reopened.get(&name, || Ok(bytes.clone())).unwrap(), bytes);
}
