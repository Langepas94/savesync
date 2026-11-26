use savesync_core::ipc::BackupPayload;

use super::Runtime;
use std::io;
impl Runtime {
    pub fn last_backup(&self) -> Option<savesync_core::ipc::BackupSummary> {
        let bytes = std::fs::read(self.state_dir.join("last-backup.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn record_backup(&self, directory: &std::path::Path) {
        use savesync_core::ipc::BackupSummary;
        let summary = BackupSummary {
            created_at: savesync_core::storage::snapshot::now_millis(),
            directory: directory.display().to_string(),
        };
        let result = (|| -> io::Result<()> {
            std::fs::create_dir_all(&self.state_dir)?;
            let temporary = self.state_dir.join("last-backup.tmp");
            std::fs::write(&temporary, serde_json::to_vec(&summary)?)?;
            std::fs::rename(temporary, self.state_dir.join("last-backup.json"))
        })();
        if let Err(error) = result {
            eprintln!("last backup metadata: {error}");
        }
    }

    /// Переносимый бэкап игры в `backups.dir` (+ retention).
    pub fn backup_game(&mut self, game_key: &str) -> Result<BackupPayload, String> {
        let Some(dir) = self.config.backups.dir.clone() else {
            return Err("папка бэкапов не задана (backups.dir)".into());
        };
        self.refresh_catalog();
        let Some(game) = self.last_scan.iter().find(|g| g.game_key == game_key) else {
            return Err("игра не найдена сканером".into());
        };
        if game.locations.is_empty() {
            return Err("сейвы ещё не созданы".into());
        }
        match savesync_core::storage::backup::create_backup(
            &game.game_key,
            &self.config.device_id,
            &game.locations,
            &dir,
        ) {
            Ok(file) => {
                let keep = self.config.backups.keep.unwrap_or(10) as usize;
                let _ = savesync_core::storage::backup::prune_backup_files(&dir, game_key, keep);
                self.record_backup(&dir);
                Ok(BackupPayload {
                    game_key: game_key.into(),
                    file: file.display().to_string(),
                })
            }
            Err(error) => Err(error.to_string()),
        }
    }
}
