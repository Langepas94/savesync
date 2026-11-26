use super::*;
#[test]
fn background_failure_is_reported_without_a_desktop() {
    let mut updates = Updates::default();
    updates.begin_check(|| Err("offline".into()));
    let event = updates
        .job
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    sender.send(event).unwrap();
    updates.job = Some(receiver);
    updates.poll();
    assert!(updates.failed);
    assert_eq!(updates.message, "offline");
    assert!(updates.job.is_none());
}
