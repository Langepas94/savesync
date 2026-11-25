use super::*;
impl Runtime {
    /// Пересканировать систему. Возвращает найденные игры.
    pub fn scan(&mut self) -> ResponsePayload {
        self.last_scan = scanner::scan(&self.manifests, self.platform, &self.roots);
        ResponsePayload::Scan(ScanPayload {
            found: self.last_scan.len(),
            games: self
                .last_scan
                .iter()
                .map(|g| ScannedGame {
                    game_key: g.game_key.clone(),
                    display_name: g.display_name.clone(),
                    locations: g.locations.len(),
                    portability: format!("{:?}", g.portable).to_lowercase(),
                })
                .collect(),
        })
    }

    pub fn status(&self) -> ResponsePayload {
        let index = savesync_core::index::Index::load(&self.state_dir.join("index.json"))
            .unwrap_or_default();
        let games = self
            .last_scan
            .iter()
            .map(|g| {
                let (versions, heads) = index
                    .game(&g.game_key)
                    .map(|gi| (gi.versions.len(), gi.heads().len()))
                    .unwrap_or((0, 0));
                let state = if heads > 1 {
                    "conflict"
                } else if versions == 0 {
                    "no_versions"
                } else {
                    "ok"
                };
                GameStatus {
                    game_key: g.game_key.clone(),
                    display_name: g.display_name.clone(),
                    versions,
                    state: state.into(),
                }
            })
            .collect();
        ResponsePayload::Status(StatusPayload {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            device_id: self.config.device_id.clone(),
            device_name: self.config.device_name.clone(),
            games,
        })
    }
}
