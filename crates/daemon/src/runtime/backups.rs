use super::*;
impl Runtime {
    /// Переносимый бэкап игры в `backups.dir` (+ retention).
    pub fn backup_game(&mut self, game_key: &str) -> ResponsePayload {
        use savesync_core::ipc::BackupPayload;
        let Some(dir) = self.config.backups.dir.clone() else {
            return ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some("папка бэкапов не задана (backups.dir)".into()),
                }],
            });
        };
        self.refresh_catalog();
        let Some(game) = self.last_scan.iter().find(|g| g.game_key == game_key) else {
            return ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some("игра не найдена сканером".into()),
                }],
            });
        };
        if game.locations.is_empty() {
            return ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some("сейвы ещё не созданы".into()),
                }],
            });
        }
        match savesync_core::backup::create_backup(
            &game.game_key,
            &self.config.device_id,
            &game.locations,
            &dir,
        ) {
            Ok(file) => {
                let keep = self.config.backups.keep.unwrap_or(10) as usize;
                let _ = savesync_core::backup::prune_backup_files(&dir, game_key, keep);
                ResponsePayload::Backup(BackupPayload {
                    game_key: game_key.into(),
                    file: file.display().to_string(),
                })
            }
            Err(e) => ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some(e.to_string()),
                }],
            }),
        }
    }
}
