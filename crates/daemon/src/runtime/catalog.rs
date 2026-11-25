use super::*;
impl Runtime {
    pub fn add_custom_game(
        &mut self,
        name: &str,
        paths: &[String],
    ) -> Result<ResponsePayload, String> {
        use savesync_core::manifest::{Os, PathRule};
        let os = match self.platform {
            Platform::Windows => Os::Windows,
            Platform::Linux => Os::Linux,
        };
        let rules = paths
            .iter()
            .map(|input| {
                let path = std::path::Path::new(input);
                if !path.is_absolute()
                    || path.parent().is_none()
                    || !(path.is_file() || path.is_dir())
                {
                    return Err(format!("Укажите существующий файл или папку: {input}"));
                }
                Ok(PathRule {
                    location: None,
                    pattern: savesync_core::paths::literal_save_pattern(path),
                    os,
                    tags: vec!["save".into()],
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut manifest = Manifest::custom(name, rules)?;
        let custom_dir = self.state_dir.join("custom");
        let (_, identity) = savesync_core::installation::resolve_custom_with_identity(
            &manifest,
            &self.manifests,
            &self.roots,
            self.platform,
        );
        let previous = if custom_dir.is_dir() && !self.hidden_games.is_empty() {
            let (custom, _) = savesync_core::manifest::load_manifest_tree(&custom_dir);
            custom.get(&manifest.slug).and_then(|previous| {
                savesync_core::installation::resolve_custom_with_identity(
                    previous,
                    &self.manifests,
                    &self.roots,
                    self.platform,
                )
                .1
            })
        } else {
            None
        };
        manifest.catalog_key = identity.or_else(|| previous.clone());
        let hidden_key = [
            Some(manifest.slug.as_str()),
            manifest.catalog_key.as_deref(),
            previous.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|key| self.hidden_games.contains_key(*key));
        manifest
            .write_custom(&custom_dir, hidden_key.is_some())
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    "Игра с таким названием уже добавлена вручную".into()
                } else {
                    format!("Не удалось сохранить правило: {error}")
                }
            })?;
        // If persisting the visibility change fails, the entry remains hidden and can be submitted again.
        if let Some(key) = hidden_key {
            let key = key.to_string();
            Ok(self.set_game_hidden(&key, false))
        } else {
            Ok(self.scan())
        }
    }

    pub fn reload_manifests(&mut self) -> ResponsePayload {
        match Self::new(self.config.clone(), self.state_dir.clone()) {
            Ok(rebuilt) => {
                self.base_manifests = rebuilt.base_manifests;
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
    pub fn set_game_hidden(&mut self, game_key: &str, hidden: bool) -> ResponsePayload {
        let mut games = self.hidden_games.clone();
        if hidden {
            let Some(game) = self.last_scan.iter().find(|g| g.game_key == game_key) else {
                return ResponsePayload::Sync(SyncPayload {
                    results: vec![SyncResult {
                        game_key: game_key.into(),
                        outcome: "error".into(),
                        detail: Some("Игра не найдена".into()),
                    }],
                });
            };
            games.insert(game_key.into(), game.display_name.clone());
        } else {
            games.remove(game_key);
        }
        let result = (|| -> io::Result<()> {
            std::fs::create_dir_all(&self.state_dir)?;
            let path = self.state_dir.join("hidden-games.json");
            let tmp = self.state_dir.join("hidden-games.json.tmp");
            std::fs::write(
                &tmp,
                serde_json::to_vec_pretty(&games).map_err(io::Error::other)?,
            )?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.hidden_games = games;
                self.scan()
            }
            Err(e) => ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: game_key.into(),
                    outcome: "error".into(),
                    detail: Some(e.to_string()),
                }],
            }),
        }
    }
    pub(super) fn refresh_catalog(&mut self) {
        self.manifests = self.base_manifests.clone();
        let directory = self.state_dir.join("custom");
        let custom = if directory.is_dir() {
            let (custom, errors) = savesync_core::manifest::load_manifest_tree(&directory);
            for error in errors {
                eprintln!("custom manifest load warning: {error}");
            }
            for manifest in custom.iter() {
                self.manifests.insert(manifest.clone());
            }
            custom
        } else {
            ManifestDb::default()
        };
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
        self.roots.steam_installations = apps
            .iter()
            .filter_map(|app| {
                use std::path::{Component, Path};
                let mut components = Path::new(&app.installdir).components();
                let (Some(Component::Normal(name)), None) = (components.next(), components.next())
                else {
                    return None;
                };
                let path = app.library.join("steamapps/common").join(name);
                path.is_dir().then_some((app.appid, path))
            })
            .collect();
        let index = savesync_core::index::Index::load(&self.state_dir.join("index.json"))
            .unwrap_or_default();
        let mut manual_keys = std::collections::HashSet::new();
        if !custom.is_empty() {
            let catalog = self.manifests.clone();
            let mut custom: Vec<_> = custom.iter().collect();
            custom.sort_by(|left, right| left.slug.cmp(&right.slug));
            for manifest in custom {
                let (mut resolved, identity) =
                    savesync_core::installation::resolve_custom_with_identity(
                        manifest,
                        &catalog,
                        &self.roots,
                        self.platform,
                    );
                if let Some(key) = identity.filter(|key| {
                    !manual_keys.contains(key)
                        && !self.hidden_games.contains_key(&manifest.slug)
                        && index.game(&manifest.slug).is_none()
                }) {
                    self.manifests.remove(&manifest.slug);
                    resolved.slug = key;
                    resolved.catalog_key = None;
                }
                manual_keys.insert(resolved.slug.clone());
                self.manifests.insert(resolved);
            }
        }
        let mut known: std::collections::HashSet<String> =
            savesync_core::steam::match_manifests(&self.manifests, &apps)
                .into_iter()
                .map(|entry| entry.game_key)
                .collect();
        known.extend(manual_keys);
        for manifest in savesync_core::installation::discover_steam_shortcuts(
            &self.manifests,
            &self.roots,
            self.platform,
        ) {
            let manual_key = savesync_core::game_key::custom_key(&manifest.name);
            if !self.hidden_games.contains_key(&manual_key) && known.insert(manifest.slug.clone()) {
                self.manifests.insert(manifest);
            }
        }
        known.extend(index.games.into_keys());
        self.last_scan = scanner::scan_catalog(&self.manifests, self.platform, &self.roots, &known);
        self.last_scan
            .retain(|g| !self.hidden_games.contains_key(&g.game_key));
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
                } else if g.locations.is_empty() {
                    "no_saves"
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
            busy: false,
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            device_id: self.config.device_id.clone(),
            device_name: self.config.device_name.clone(),
            games,
            hidden_games: self
                .hidden_games
                .iter()
                .map(|(key, name)| GameStatus {
                    game_key: key.clone(),
                    display_name: name.clone(),
                    versions: 0,
                    state: "hidden".into(),
                })
                .collect(),
        })
    }
}
