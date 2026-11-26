use super::*;
#[test]
fn invalid_update_preserves_installed_database_and_custom_rules() {
    let root = std::env::temp_dir().join(format!("ss-manifest-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("source/manifests")).unwrap();
    std::fs::create_dir_all(root.join("state/custom")).unwrap();
    std::fs::write(root.join("state/custom/mine.yaml"), "untouched").unwrap();
    std::fs::write(
        root.join("source/manifests/game.yaml"),
        "schema: 1\nname: Game\nslug: game\npaths:\n - pattern: '<home>/saves/*.sav'\n",
    )
    .unwrap();
    let destination = root.join("manifests");
    assert_eq!(install(&root.join("source"), &destination).unwrap(), 1);
    let previous = std::fs::read(destination.join("game.yaml")).unwrap();
    std::fs::write(root.join("source/manifests/broken.yaml"), "not a manifest").unwrap();
    assert!(install(&root.join("source"), &destination).is_err());
    assert_eq!(
        std::fs::read(destination.join("game.yaml")).unwrap(),
        previous
    );
    assert_eq!(
        std::fs::read_to_string(root.join("state/custom/mine.yaml")).unwrap(),
        "untouched"
    );
    std::fs::remove_dir_all(root).unwrap();
}
