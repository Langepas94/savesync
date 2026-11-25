use super::*;
impl<B: Backend> Engine<B> {
    /// Есть ли у игры локальные версии (устройство уже синхронизировало её).
    pub fn has_local_versions(&self, game_key: &str) -> io::Result<bool> {
        Ok(self
            .load_local()?
            .game(game_key)
            .map(|g| !g.versions.is_empty())
            .unwrap_or(false))
    }

    /// Есть ли у игры версии на бэкенде.
    pub fn has_remote_versions(&self, game_key: &str) -> Result<bool, EngineError> {
        Ok(self
            .load_remote()?
            .game(game_key)
            .map(|g| !g.versions.is_empty())
            .unwrap_or(false))
    }

    /// Захват/продление лока игры в общем индексе (plan §2.1).
    /// false — игру держит другое устройство со свежим heartbeat.
    pub fn lock_game(&self, game_key: &str) -> Result<bool, EngineError> {
        let mut remote = self.load_remote()?;
        let acquired = remote.acquire_lock(game_key, &self.device_id, now_millis());
        if acquired {
            self.save_remote(&remote)?;
        }
        Ok(acquired)
    }

    /// Снятие своего лока (чужой не трогаем).
    pub fn unlock_game(&self, game_key: &str) -> Result<(), EngineError> {
        let mut remote = self.load_remote()?;
        remote.release_lock(game_key, &self.device_id);
        self.save_remote(&remote)
    }

    /// Кто держит свежий лок игры, если это не мы.
    pub fn foreign_lock(&self, game_key: &str) -> Result<Option<String>, EngineError> {
        let remote = self.load_remote()?;
        Ok(remote
            .lock_holder(game_key, now_millis())
            .filter(|l| l.device != self.device_id)
            .map(|l| l.device.clone()))
    }
}
