#[test]
fn bundled_manifests_are_valid() {
    let database = super::load();
    assert!(!database.is_empty());
    for manifest in database.iter() {
        assert!(manifest.validate().is_empty(), "{}", manifest.slug);
    }
}
