use super::*;
impl Runtime {
    pub fn reload_manifests(&mut self) -> ResponsePayload {
        match Self::new(self.config.clone(), self.state_dir.clone()) {
            Ok(rebuilt) => {
                self.manifests = rebuilt.manifests;
                self.scan()
            }
            Err(error) => ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: "manifests".into(),
                    outcome: "error".into(),
                    detail: Some(error.to_string()),
                }],
            }),
        }
    }
    pub(super) fn refresh_catalog(&mut self) {
        let (apps, libraries, diagnostics) = savesync_core::steam::discovery::discover(
            &mut self.manifests,
            &self.roots.steam_libraries,
        );
        for message in diagnostics {
            eprintln!("Steam discovery: {message}");
        }
        let mut paths = std::collections::HashSet::new();
        self.roots.steam_libraries = self
            .roots
            .steam_libraries
            .iter()
            .chain(libraries.iter())
            .map(|path| savesync_core::paths::canonical_path(path))
            .filter(|path| paths.insert(path.clone()))
            .collect();
        let mut known: std::collections::HashSet<String> =
            savesync_core::steam::match_manifests(&self.manifests, &apps)
                .into_iter()
                .map(|entry| entry.game_key)
                .collect();
        if let Ok(index) = savesync_core::index::Index::load(&self.state_dir.join("index.json")) {
            known.extend(index.games.into_keys());
        }
        self.last_scan = scanner::scan_catalog(&self.manifests, self.platform, &self.roots, &known);
    }
    /// Пересканировать систему. Возвращает найденные игры.
    pub fn scan(&mut self) -> ResponsePayload {
        self.refresh_catalog();
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
