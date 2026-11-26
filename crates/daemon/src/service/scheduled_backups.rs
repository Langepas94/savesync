use savesync_core::ipc::BackupBatchPayload;

use super::Runtime;
use std::io;
impl Runtime {
    /// Авто-бэкапы по расписанию (plan §2.2): переносимый бэкап каждой
    /// найденной игры в `backups.dir` + retention. Число созданных файлов.
    pub fn run_auto_backup(&mut self) -> io::Result<usize> {
        let batch = self.backup_all();
        if batch.created > 0 || batch.errors.is_empty() {
            self.mark_auto_backup_done();
        }
        if batch.errors.is_empty() {
            Ok(batch.created)
        } else {
            Err(io::Error::other(batch.errors.join("\n")))
        }
    }

    pub fn backup_all(&mut self) -> BackupBatchPayload {
        let Some(dir) = self.config.backups.dir.clone() else {
            return BackupBatchPayload {
                created: 0,
                directory: String::new(),
                errors: vec!["Укажите папку бэкапов".into()],
            };
        };
        self.refresh_catalog();
        let keep = self.config.backups.keep.unwrap_or(10) as usize;
        let mut created = 0;
        let mut errors = Vec::new();
        for g in &self.last_scan {
            if g.locations.is_empty() {
                continue;
            }
            match savesync_core::storage::backup::create_backup(
                &g.game_key,
                &self.config.device_id,
                &g.locations,
                &dir,
            ) {
                Ok(_) => {
                    created += 1;
                    if let Err(error) =
                        savesync_core::storage::backup::prune_backup_files(&dir, &g.game_key, keep)
                    {
                        errors.push(format!("{}: {error}", g.display_name));
                    }
                }
                Err(error) => errors.push(format!("{}: {error}", g.display_name)),
            }
        }
        if created > 0 {
            self.record_backup(&dir);
        }
        BackupBatchPayload {
            created,
            directory: dir.display().to_string(),
            errors,
        }
    }

    /// Пора ли делать авто-бэкап (интервал из конфига прошёл).
    pub fn auto_backup_due(&self) -> bool {
        let Some(hours) = self.config.backups.auto_interval_hours else {
            return false;
        };
        if self.config.backups.dir.is_none() {
            return false;
        }
        let stamp = self.state_dir.join("last_auto_backup");
        let last = std::fs::read_to_string(&stamp)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        savesync_core::storage::snapshot::now_millis().saturating_sub(last)
            >= u64::from(hours) * 3_600_000
    }

    /// Отметить время последнего авто-бэкапа.
    fn mark_auto_backup_done(&self) {
        let _ = std::fs::create_dir_all(&self.state_dir);
        let _ = std::fs::write(
            self.state_dir.join("last_auto_backup"),
            savesync_core::storage::snapshot::now_millis().to_string(),
        );
    }
}
