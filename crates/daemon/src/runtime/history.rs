use super::*;
impl Runtime {
    /// История версий игры из локального индекса (для карточки игры и
    /// конфликт-резолвера).
    pub fn history(&self, game_key: &str) -> ResponsePayload {
        use savesync_core::ipc::{HistoryPayload, VersionEntry};
        let index = savesync_core::index::Index::load(&self.state_dir.join("index.json"))
            .unwrap_or_default();
        let applied: std::collections::BTreeMap<String, String> =
            std::fs::read(self.state_dir.join("applied.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();

        let (mut versions, heads) = match index.game(game_key) {
            Some(g) => (
                g.versions
                    .iter()
                    .map(|v| {
                        let (channel, file_count) = savesync_core::engine::snapshot_details(
                            &self.state_dir,
                            game_key,
                            &v.id,
                        );
                        VersionEntry {
                            id: v.id.clone(),
                            parent: v.parent.clone(),
                            created_at: v.created_at,
                            device: v.device.clone(),
                            device_name: v.device_name.clone(),
                            hash: v.hash.clone(),
                            channel,
                            file_count,
                        }
                    })
                    .collect::<Vec<_>>(),
                g.heads().iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        versions.sort_unstable_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        ResponsePayload::History(HistoryPayload {
            game_key: game_key.to_string(),
            install_paths: self
                .manifests
                .get(game_key)
                .into_iter()
                .flat_map(|manifest| {
                    manifest.installed_at.iter().chain(
                        manifest
                            .ids
                            .steam
                            .iter()
                            .filter_map(|id| id.parse::<u32>().ok())
                            .filter_map(|id| self.roots.steam_installations.get(&id)),
                    )
                })
                .map(|path| path.display().to_string())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
            local_files: self
                .last_scan
                .iter()
                .find(|game| game.game_key == game_key)
                .into_iter()
                .flat_map(|game| &game.locations)
                .flat_map(|location| &location.hits)
                .filter_map(|path| {
                    let metadata = std::fs::metadata(path).ok()?;
                    if !metadata.is_file() {
                        return None;
                    }
                    Some(savesync_core::ipc::LocalSaveFile {
                        path: path.to_string_lossy().into_owned(),
                        size: metadata.len(),
                        modified_at: metadata
                            .modified()
                            .ok()
                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|time| time.as_millis().min(u64::MAX as u128) as u64),
                    })
                })
                .collect(),
            versions,
            applied: applied.get(game_key).cloned(),
            heads,
        })
    }

    /// Применить конкретную версию (откат / выбор ветки конфликта).
    /// Пробует каналы по очереди — снапшот может лежать не во всех.
    pub fn restore(&mut self, game_key: &str, version_id: &str) -> ResponsePayload {
        let manifest = match self.manifests.get(game_key) {
            Some(m) => m.clone(),
            None => {
                return ResponsePayload::Sync(SyncPayload {
                    results: vec![SyncResult {
                        game_key: game_key.into(),
                        outcome: "error".into(),
                        detail: Some("нет манифеста".into()),
                    }],
                })
            }
        };
        let engines = match self.engines() {
            Ok(e) => e,
            Err(msg) => {
                return ResponsePayload::Sync(SyncPayload {
                    results: vec![SyncResult {
                        game_key: game_key.into(),
                        outcome: "error".into(),
                        detail: Some(msg),
                    }],
                })
            }
        };
        let mut results = Vec::new();
        for engine in &engines {
            let channel = engine.backend_name();
            match engine.pull_version(&manifest, &self.roots, self.platform, version_id) {
                Ok(PullOutcome::Applied { version, .. }) => results.push(SyncResult {
                    game_key: game_key.into(),
                    outcome: "applied".into(),
                    detail: Some(format!("[{channel}] {version}")),
                }),
                Ok(PullOutcome::UpToDate) => results.push(SyncResult {
                    game_key: game_key.into(),
                    outcome: "up_to_date".into(),
                    detail: Some(format!("[{channel}]")),
                }),
                Ok(other) => results.push(SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some(format!("[{channel}] {other:?}")),
                }),
                Err(e) => results.push(SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some(format!("[{channel}] {e}")),
                }),
            }
            if results
                .last()
                .is_some_and(|result| matches!(result.outcome.as_str(), "applied" | "up_to_date"))
            {
                break;
            }
        }
        if results.iter().any(|result| result.outcome == "applied") {
            self.refresh_catalog();
        }
        ResponsePayload::Sync(SyncPayload { results })
    }
}
