#[cfg(unix)]
#[test]
fn other_unprivileged_users_are_rejected() {
    assert!(super::allowed_uid(1000, 1000));
    assert!(super::allowed_uid(1000, 0));
    assert!(!super::allowed_uid(1000, 1001));
}
#[cfg(windows)]
#[test]
fn current_process_has_stable_token_identity() {
    unsafe {
        let process = windows_sys::Win32::System::Threading::GetCurrentProcess();
        assert_eq!(
            super::user_sid(process).unwrap(),
            super::user_sid(process).unwrap()
        );
    }
}
