use super::*;
fn metadata(tag: &str) -> serde_json::Value {
    serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false,"html_url":format!("https://github.com/Langepas94/savesync/releases/tag/{tag}"),"assets":[{"id":1,"name":INSTALLER,"size":100},{"id":2,"name":format!("{INSTALLER}.sha256"),"size":87}]})
}
#[test]
fn stable_semver_check_rejects_old_and_prerelease() {
    assert!(
        select(&serde_json::to_vec(&metadata("v1.2.0")).unwrap(), "1.1.0")
            .unwrap()
            .is_some()
    );
    assert!(
        select(&serde_json::to_vec(&metadata("v1.0.0")).unwrap(), "1.1.0")
            .unwrap()
            .is_none()
    );
    assert!(select(
        &serde_json::to_vec(&metadata("v2.0.0-beta.1")).unwrap(),
        "1.1.0"
    )
    .unwrap()
    .is_none());
}
#[test]
fn ambiguous_or_unsafe_release_assets_are_rejected() {
    let mut value = metadata("v2.0.0");
    let duplicate = value["assets"][0].clone();
    value["assets"].as_array_mut().unwrap().push(duplicate);
    assert!(select(&serde_json::to_vec(&value).unwrap(), "1.0.0").is_err());
    let mut value = metadata("v2.0.0");
    value["html_url"] = "https://example.com/install.exe".into();
    assert!(select(&serde_json::to_vec(&value).unwrap(), "1.0.0").is_err());
}
#[test]
fn checksum_is_bound_to_expected_installer_name() {
    let hash = "a".repeat(64);
    assert_eq!(
        checksum(format!("{hash}  {INSTALLER}\n").as_bytes()).unwrap(),
        hash
    );
    assert!(checksum(format!("{hash} other.exe").as_bytes()).is_err());
    assert!(checksum(b"bad checksum").is_err());
}
#[cfg(windows)]
#[test]
fn unsigned_file_is_never_accepted_as_installer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("savesync-setup.exe");
    std::fs::write(&path, b"unsigned data").unwrap();
    assert!(verify_installer(&path, &semver::Version::new(1, 0, 0)).is_err());
}
