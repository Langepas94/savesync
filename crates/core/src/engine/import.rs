use super::*;
impl<B: Backend> Engine<B> {
    /// Импорт переносимого бэкапа `*.savesync-backup` (plan §2.2):
    /// файл регистрируется как новая версия в линии игры (видна в
    /// истории, можно откатиться через pull_version). Само применение
    /// сейвов — отдельным pull.
    pub fn import_backup(&self, path: &Path) -> Result<ImportOutcome, EngineError> {
        if !snapshot::verify(path)? {
            return Err(EngineError::CorruptedSnapshot(path.display().to_string()));
        }
        let snap_manifest = snapshot::read_manifest(path)?;
        let game_key = snap_manifest.game_key.clone();

        let mut local = self.load_local()?;
        let head = local.game(&game_key).and_then(|g| g.head().cloned());
        // Тот же контент от той же базы уже есть — дубликат.
        if head.as_ref().map(|h| h.hash.as_str()) == Some(snap_manifest.content_hash.as_str()) {
            return Ok(ImportOutcome::Duplicate(head.unwrap().id));
        }

        let stamp = now_millis();
        let id = version_id(stamp, &self.device_id, &snap_manifest.content_hash);
        let version = Version {
            parent: head.map(|h| h.id),
            merged_parents: Vec::new(),
            id: id.clone(),
            created_at: stamp,
            device: format!("{}(import)", snap_manifest.device_id),
            hash: snap_manifest.content_hash,
        };

        let data = std::fs::read(path)?;
        self.cache_snapshot(&game_key, &id, &data)?;
        self.backend.put_snapshot(&game_key, &id, &data)?;

        local.add_version(&game_key, version.clone());
        self.save_local(&local)?;

        self.publish_local(&local)?;

        Ok(ImportOutcome::Registered(version))
    }
}
