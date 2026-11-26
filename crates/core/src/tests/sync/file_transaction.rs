use super::*;
struct Fixture {
    root: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ss-file-tx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("index.json"), "old-index").unwrap();
        std::fs::write(state.join("applied.json"), "old-applied").unwrap();
        Self { root, state }
    }
    fn plan(&self) -> Transaction {
        for (name, contents) in [
            ("a.sav", "old-a"),
            ("b.sav", "old-b"),
            ("stale.sav", "old-stale"),
            ("new-a", "new-a"),
            ("new-b", "new-b"),
            ("new-c", "new-c"),
        ] {
            std::fs::write(self.root.join(name), contents).unwrap();
        }
        Transaction::prepare(
            &self.state,
            "game",
            vec![
                (self.root.join("a.sav"), self.root.join("new-a")),
                (self.root.join("b.sav"), self.root.join("new-b")),
                (self.root.join("c.sav"), self.root.join("new-c")),
            ],
            vec![self.root.join("stale.sav")],
        )
        .unwrap()
    }
    fn assert_original(&self) {
        for (name, contents) in [
            ("a.sav", "old-a"),
            ("b.sav", "old-b"),
            ("stale.sav", "old-stale"),
        ] {
            assert_eq!(
                std::fs::read_to_string(self.root.join(name)).unwrap(),
                contents
            );
        }
        assert!(!self.root.join("c.sav").exists());
        assert_eq!(
            std::fs::read_to_string(self.state.join("index.json")).unwrap(),
            "old-index"
        );
        assert_eq!(
            std::fs::read_to_string(self.state.join("applied.json")).unwrap(),
            "old-applied"
        );
        assert!(!self.state.join("restore-journal.json").exists());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn metadata_failure_rolls_back_replacements_removals_and_new_files() {
    let fixture = Fixture::new("metadata");
    let transaction = fixture.plan();
    let result = transaction.commit(|| {
        std::fs::write(fixture.state.join("index.json"), "new-index")?;
        std::fs::write(fixture.state.join("applied.json"), "new-applied")?;
        Err::<(), _>(io::Error::other("injected metadata failure").into())
    });
    assert!(result.is_err());
    fixture.assert_original();
}
#[test]
fn file_commit_failure_rolls_back_previously_replaced_files() {
    let fixture = Fixture::new("write");
    let transaction = fixture.plan();
    std::fs::remove_file(transaction.journal.entries[1].prepared.as_ref().unwrap()).unwrap();
    assert!(transaction.commit(|| Ok(())).is_err());
    fixture.assert_original();
}
#[test]
fn interrupted_restore_is_recovered_with_its_old_metadata() {
    let fixture = Fixture::new("crash");
    let mut transaction = fixture.plan();
    transaction.apply().unwrap();
    std::fs::write(fixture.state.join("index.json"), "new-index").unwrap();
    std::mem::forget(transaction);
    recover_pending(&fixture.state).unwrap();
    fixture.assert_original();
}
#[test]
fn committed_journal_cleanup_keeps_new_files_and_metadata() {
    let fixture = Fixture::new("committed");
    let mut transaction = fixture.plan();
    transaction.apply().unwrap();
    std::fs::write(fixture.state.join("applied.json"), "new-applied").unwrap();
    transaction.journal.committed = true;
    transaction.save_journal().unwrap();
    std::mem::forget(transaction);
    recover_pending(&fixture.state).unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("a.sav")).unwrap(),
        "new-a"
    );
    assert!(!fixture.root.join("stale.sav").exists());
    assert_eq!(
        std::fs::read_to_string(fixture.state.join("applied.json")).unwrap(),
        "new-applied"
    );
    assert!(!fixture.state.join("restore-journal.json").exists());
}
