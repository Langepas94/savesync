use super::*;
use savesync_core::ipc::{SyncPayload, SyncResult};
#[test]
fn operation_errors_cannot_be_reported_as_success() {
    for outcome in ["error", "locked", "conflict", "skipped"] {
        let payload = ResponsePayload::Sync(SyncPayload {
            results: vec![SyncResult {
                game_key: "game".into(),
                outcome: outcome.into(),
                detail: Some("backend offline".into()),
            }],
        });
        for method in [
            Method::Restore {
                game_key: "game".into(),
                version_id: "v1".into(),
            },
            Method::Backup {
                game_key: "game".into(),
            },
        ] {
            assert!(validate_result(&method, payload.clone())
                .unwrap_err()
                .contains(outcome));
        }
    }
}
#[test]
fn hiding_game_accepts_scan_response() {
    let method = Method::SetGameHidden {
        game_key: "game".into(),
        hidden: true,
    };
    let payload = ResponsePayload::Scan(savesync_core::ipc::ScanPayload {
        found: 0,
        games: vec![],
    });
    assert!(validate_result(&method, payload).is_ok());
}
#[test]
fn empty_restore_and_wrong_payload_are_rejected() {
    let payload = ResponsePayload::Sync(SyncPayload { results: vec![] });
    assert!(validate_result(
        &Method::Restore {
            game_key: "g".into(),
            version_id: "v1".into()
        },
        payload.clone()
    )
    .is_err());
    assert!(validate_result(&Method::GetConfig, payload).is_err());
}

#[test]
fn restore_can_succeed_using_a_later_channel() {
    let method = Method::Restore {
        game_key: "game".into(),
        version_id: "v1".into(),
    };
    let results = vec![
        SyncResult {
            game_key: "game".into(),
            outcome: "error".into(),
            detail: Some("copy unavailable".into()),
        },
        SyncResult {
            game_key: "game".into(),
            outcome: "applied".into(),
            detail: None,
        },
    ];
    assert!(validate_result(&method, ResponsePayload::Sync(SyncPayload { results })).is_ok());
}
