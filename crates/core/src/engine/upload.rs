use super::*;
impl<B: Backend> Engine<B> {
    /// Выгрузка: снапшот → бэкенд → обновление локального и общего индексов.
    pub fn push(
        &self,
        game_key: &str,
        locations: &[SaveLocation],
    ) -> Result<PushOutcome, EngineError> {
        let _guard = self.state_guard()?;
        // Игра запущена на другом устройстве — ждём (plan §2.1).
        if let Some(holder) = self.foreign_lock(game_key)? {
            return Ok(PushOutcome::Locked(holder));
        }

        self.cache_remote_history(&self.load_remote()?, game_key)?;
        let tmp_dir = self.state_dir.join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let component = crate::storage_paths::game_component(game_key)?;
        let tmp_snap = tmp_dir.join(format!("{component}.zip"));

        let manifest = snapshot::create(game_key, &self.device_id, locations, &tmp_snap)?;
        if manifest
            .locations
            .iter()
            .all(|location| location.files.is_empty())
        {
            std::fs::remove_file(&tmp_snap).ok();
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "no save files to upload").into(),
            );
        }

        let mut local = self.load_local()?;
        let applied = self.applied_of(game_key);
        let local_head = local.game(game_key).and_then(|g| {
            applied
                .as_deref()
                .and_then(|id| g.get(id))
                .or_else(|| g.head())
                .cloned()
        });
        if local_head.as_ref().map(|h| h.hash.as_str()) == Some(manifest.content_hash.as_str()) {
            // Local deduplication must not suppress delivery to a second channel
            // or a retry after an index upload failed.
            let version = local_head.as_ref().unwrap();
            if !self.cache_path(game_key, &version.id)?.is_file() {
                self.cache_snapshot(game_key, &version.id, &std::fs::read(&tmp_snap)?)?;
            }
            self.publish_local(&local)?;
            std::fs::remove_file(&tmp_snap).ok();
            return Ok(PushOutcome::Unchanged);
        }

        let id = version_id(manifest.created_at, &self.device_id, &manifest.content_hash);
        let version = Version {
            parent: local_head.map(|h| h.id),
            merged_parents: Vec::new(),
            id: id.clone(),
            created_at: manifest.created_at,
            device: self.device_id.clone(),
            device_name: self.device_name.clone(),
            hash: manifest.content_hash,
        };

        let data = std::fs::read(&tmp_snap)?;
        self.cache_snapshot(game_key, &id, &data)?;
        self.backend.put_snapshot(game_key, &id, &data)?;
        std::fs::remove_file(&tmp_snap).ok();

        local.add_version(game_key, version.clone());
        self.save_local(&local)?;
        self.mark_applied(game_key, &id)?;

        // Общий индекс: многосторонний merge по id версий.
        self.publish_local(&local)?;

        Ok(PushOutcome::Uploaded(version))
    }
}
