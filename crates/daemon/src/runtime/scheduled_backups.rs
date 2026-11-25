use super::*;
impl Runtime {
    /// Авто-бэкапы по расписанию (plan §2.2): переносимый бэкап каждой
    /// найденной игры в `backups.dir` + retention. Число созданных файлов.
    pub fn run_auto_backup(&mut self) -> io::Result<usize> {
        let Some(dir) = self.config.backups.dir.clone() else {
            return Ok(0);
        };
        self.refresh_catalog();
        let keep = self.config.backups.keep.unwrap_or(10) as usize;
        let mut created = 0;
        let games = self.last_scan.clone();
        for g in &games {
            if g.locations.is_empty() {
                continue;
            }
            savesync_core::backup::create_backup(
                &g.game_key,
                &self.config.device_id,
                &g.locations,
                &dir,
            )?;
            savesync_core::backup::prune_backup_files(&dir, &g.game_key, keep)?;
            created += 1;
        }
        Ok(created)
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
        savesync_core::snapshot::now_millis().saturating_sub(last) >= u64::from(hours) * 3_600_000
    }

    /// Отметить время последнего авто-бэкапа.
    pub fn mark_auto_backup_done(&self) {
        let _ = std::fs::create_dir_all(&self.state_dir);
        let _ = std::fs::write(
            self.state_dir.join("last_auto_backup"),
            savesync_core::snapshot::now_millis().to_string(),
        );
    }
}
