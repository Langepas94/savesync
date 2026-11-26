use super::*;
#[test]
fn menu_dispatches_only_known_actions() {
    let id = MenuId::new("sync");
    let ids = [(id.clone(), Action::Sync)];
    assert_eq!(dispatch(&ids, &id), Some(Action::Sync));
    assert_eq!(dispatch(&ids, &MenuId::new("other")), None);
}
