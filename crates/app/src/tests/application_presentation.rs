use super::*;

#[test]
fn civil_from_days_matches_known_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    // 2026-01-01 = 20454 (2025 не високосный), +328 дней до 25 ноября.
    assert_eq!(civil_from_days(20_782), (2026, 11, 25));
}

#[test]
fn fmt_ts_zero_is_epoch() {
    assert_eq!(fmt_ts(0), "01.01.1970, 00:00 UTC");
}

#[test]
fn sync_summary_formats_outcomes() {
    let p = ResponsePayload::Sync(savesync_core::ipc::SyncPayload {
        results: vec![SyncResult {
            game_key: "elden-ring".into(),
            outcome: "uploaded".into(),
            detail: Some("[fs] uploaded".into()),
        }],
    });
    let s = sync_summary(&p);
    assert!(s.contains("elden-ring"));
    assert!(s.contains("uploaded"));
}
