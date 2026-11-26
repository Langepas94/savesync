use savesync_decky_bridge::{GameSyncStatus, SyncState};

#[test]
fn status_roundtrips_json() {
    let status = GameSyncStatus {
        game_key: "elden-ring".into(),
        display_name: "Elden Ring".into(),
        state: SyncState::Conflict,
    };
    let json = serde_json::to_string(&status).unwrap();
    let back: GameSyncStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(back.state, SyncState::Conflict);
}
