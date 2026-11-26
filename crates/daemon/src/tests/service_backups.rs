use super::*;

#[test]
fn changed_backup_directory_applies_to_the_next_batch() {
    let world = World::new("backup-directory");
    let mut pc = world.device("pc");
    let initial = world.dir.join("first");
    let next = world.dir.join("second");
    let manifests = world.dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("test-game.yaml"),
        "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: '<home>/game/saves'\n",
    )
    .unwrap();
    pc.config.manifests_dir = Some(manifests);
    pc.config.backups.dir = Some(initial.clone());
    let backup = pc.backup_all();
    assert_eq!(backup.created, 1);
    assert!(backup.errors.is_empty());
    let mut config = pc.config.clone();
    config.backups.dir = Some(next.clone());
    assert!(pc.save_config(config).is_ok());
    assert!(next.is_dir());
    assert_eq!(pc.run_auto_backup().unwrap(), 1);
    assert_eq!(PathBuf::from(pc.last_backup().unwrap().directory), next);
    for directory in [initial, next] {
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 1);
    }
}
