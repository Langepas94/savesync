use super::*;
impl<B: Backend> Engine<B> {
    pub(super) fn local_index_path(&self) -> PathBuf {
        self.state_dir.join("index.json")
    }

    /// Локальное (per-device) состояние: какая версия применена на этом
    /// устройстве. Хранится отдельно от индекса, чтобы не протекать
    /// в общий index.json бэкенда.
    pub(super) fn applied_path(&self) -> PathBuf {
        self.state_dir.join("applied.json")
    }

    pub(super) fn load_applied(&self) -> io::Result<std::collections::BTreeMap<String, String>> {
        match std::fs::read(self.applied_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(e),
        }
    }

    pub(super) fn mark_applied(&self, game_key: &str, version: &str) -> io::Result<()> {
        let mut applied = self.load_applied()?;
        applied.insert(game_key.to_string(), version.to_string());
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::write(
            self.applied_path(),
            serde_json::to_vec_pretty(&applied)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        )
    }

    pub(super) fn applied_of(&self, game_key: &str) -> Option<String> {
        self.load_applied().ok()?.get(game_key).cloned()
    }

    pub(super) fn load_local(&self) -> io::Result<Index> {
        Index::load(&self.local_index_path())
    }

    pub(super) fn save_local(&self, index: &Index) -> io::Result<()> {
        std::fs::create_dir_all(&self.state_dir)?;
        index.save(&self.local_index_path())
    }

    pub(super) fn load_remote(&self) -> Result<Index, EngineError> {
        match self.backend.get_index()? {
            None => Ok(Index::new()),
            Some(bytes) => serde_json::from_slice(&bytes).map_err(EngineError::BadRemoteIndex),
        }
    }

    pub(super) fn save_remote(&self, index: &Index) -> Result<(), EngineError> {
        let bytes = serde_json::to_vec(index).map_err(EngineError::BadRemoteIndex)?;
        self.backend.put_index(&bytes)?;
        Ok(())
    }

    pub(super) fn cache_path(&self, game: &str, version: &str) -> io::Result<PathBuf> {
        crate::storage_paths::validate_version(version)?;
        Ok(self
            .state_dir
            .join("snapshots")
            .join(crate::storage_paths::game_component(game)?)
            .join(format!("{version}.zip")))
    }

    pub(super) fn cache_snapshot(&self, game: &str, version: &str, data: &[u8]) -> io::Result<()> {
        let path = self.cache_path(game, version)?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, data)
    }

    /// Backfill snapshots from the source channel when upgrading an older index
    /// that predates the persistent cache.
    pub(super) fn cache_remote_history(
        &self,
        remote: &Index,
        game: &str,
    ) -> Result<(), EngineError> {
        if let Some(history) = remote.game(game) {
            for version in &history.versions {
                if !self.cache_path(game, &version.id)?.exists() {
                    match self.backend.get_snapshot(game, &version.id) {
                        Ok(bytes) => self.cache_snapshot(game, &version.id, &bytes)?,
                        Err(error) => {
                            eprintln!("history cache: {game}/{} unavailable: {error}", version.id)
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Publish archives before advertising their IDs on a channel.
    pub(super) fn publish_local(&self, local: &Index) -> Result<(), EngineError> {
        let mut remote = self.load_remote()?;
        for (game, history) in &local.games {
            for version in &history.versions {
                if remote.game(game).and_then(|g| g.get(&version.id)).is_some() {
                    continue;
                }
                match std::fs::read(self.cache_path(game, &version.id)?) {
                    Ok(data) => {
                        self.backend.put_snapshot(game, &version.id, &data)?;
                        remote.add_version(game, version.clone());
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.into()),
                }
            }
        }
        self.save_remote(&remote)
    }
}
