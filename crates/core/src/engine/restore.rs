use super::*;
impl<B: Backend> Engine<B> {
    /// Загрузка последней версии (по конфликт-политике игры).
    pub fn pull(
        &self,
        manifest: &Manifest,
        roots: &Roots,
        platform: Platform,
    ) -> Result<PullOutcome, EngineError> {
        let game_key = manifest.slug.as_str();
        // Игра запущена на другом устройстве — не затираем её сейвы.
        if let Some(holder) = self.foreign_lock(game_key)? {
            return Ok(PullOutcome::Locked(holder));
        }

        let mut local = self.load_local()?;
        let remote = self.load_remote()?;
        self.cache_remote_history(&remote, game_key)?;
        local.merge(&remote);
        self.save_local(&local)?;

        let Some(game) = local.game(game_key) else {
            return Ok(PullOutcome::NoRemoteVersion);
        };
        if game.versions.is_empty() {
            return Ok(PullOutcome::NoRemoteVersion);
        }

        let heads = game.heads();
        let target = match heads.len() {
            0 => return Ok(PullOutcome::NoRemoteVersion),
            1 => heads[0].id.clone(),
            _ => match game.policy {
                // Ask — решение в UI; keep-both не требует действий на
                // уровне индекса: проигравшая ветка и так сохраняется.
                ConflictPolicy::Ask | ConflictPolicy::KeepBoth => {
                    return Ok(PullOutcome::Conflict(
                        heads.iter().map(|h| h.id.clone()).collect(),
                    ))
                }
                ConflictPolicy::LatestWins => heads
                    .iter()
                    .max_by_key(|h| h.created_at)
                    .unwrap()
                    .id
                    .clone(),
            },
        };

        self.pull_version(manifest, roots, platform, &target)
    }

    /// Применение конкретной версии (откат, выбор ветки конфликта).
    pub fn pull_version(
        &self,
        manifest: &Manifest,
        roots: &Roots,
        platform: Platform,
        version_id_: &str,
    ) -> Result<PullOutcome, EngineError> {
        let game_key = manifest.slug.as_str();
        crate::storage_paths::validate_version(version_id_)?;
        let component = crate::storage_paths::game_component(game_key)?;
        if manifest.portability.status == crate::manifest::PortabilityStatus::Blocked {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "game saves are not portable",
            )
            .into());
        }
        if let Some(holder) = self.foreign_lock(game_key)? {
            return Ok(PullOutcome::Locked(holder));
        }
        let mut local = self.load_local()?;

        let remote = self.load_remote()?;
        local.merge(&remote);
        let heads: Vec<String> = local
            .game(game_key)
            .map(|g| g.heads().iter().map(|v| v.id.clone()).collect())
            .unwrap_or_default();
        if self.applied_of(game_key).as_deref() == Some(version_id_) && heads.len() <= 1 {
            return Ok(PullOutcome::UpToDate);
        }

        let bytes = self.backend.get_snapshot(game_key, version_id_)?;
        let tmp_dir = self.state_dir.join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let snap_path = tmp_dir.join(format!("{component}-{version_id_}.zip"));
        std::fs::write(&snap_path, bytes)?;

        if !snapshot::verify(&snap_path)? {
            std::fs::remove_file(&snap_path).ok();
            return Err(EngineError::CorruptedSnapshot(version_id_.into()));
        }
        let snap_manifest = snapshot::read_manifest(&snap_path)?;
        if snap_manifest.game_key != game_key
            || local
                .game(game_key)
                .and_then(|g| g.get(version_id_))
                .is_none_or(|v| v.hash != snap_manifest.content_hash)
        {
            return Err(EngineError::CorruptedSnapshot(version_id_.into()));
        }

        self.cache_snapshot(game_key, version_id_, &std::fs::read(&snap_path)?)?;

        // Базы локаций на ЭТОМ устройстве: паттерн -> резолв -> база.
        let mut skipped = Vec::new();
        let mut targets = Vec::new();
        for loc in &snap_manifest.locations {
            match destinations::resolve(manifest, roots, platform, loc) {
                Some(target) => targets.push(target),
                None => skipped.push(loc.pattern.clone()),
            }
        }
        if !skipped.is_empty() || targets.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "no complete destination mapping for snapshot: {}",
                    skipped.join(", ")
                ),
            )
            .into());
        }

        // Авто-бэкап текущего состояния перед затиранием (plan §2.1).
        let mut destination_names = std::collections::HashSet::new();
        for (location, base) in &targets {
            snapshot::validate_destination(location, base)?;
            for file in &location.files {
                let key = base
                    .join(&file.rel_path)
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase();
                if !destination_names.insert(key) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "colliding restore destinations",
                    )
                    .into());
                }
            }
        }
        let stamp = snapshot::now_millis();
        for (location, base) in &targets {
            for file in &location.files {
                let current = base.join(&file.rel_path);
                if current.is_file() {
                    let backup = self
                        .state_dir
                        .join("backups")
                        .join(&component)
                        .join(stamp.to_string())
                        .join(sanitize(base))
                        .join(&file.rel_path);
                    if let Some(parent) = backup.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(&current, &backup)?;
                }
            }
        }
        for (loc, base) in &targets {
            let source = snap_manifest
                .locations
                .iter()
                .find(|source| source.archive_dir == loc.archive_dir)
                .unwrap();
            snapshot::extract_location_mapped(&snap_path, source, loc, base)?;
        }
        let _ = crate::backup::prune_dir_backups(
            &self.state_dir,
            game_key,
            self.backup_retention.max(1) as usize,
        );
        std::fs::remove_file(&snap_path).ok();

        // A conflict choice is a new merge commit, preserving every old
        // snapshot while ensuring the next sync has a single head.
        let applied = if heads.len() > 1 {
            let selected = local
                .game(game_key)
                .and_then(|g| g.get(version_id_))
                .unwrap();
            let mut stamp = now_millis().max(
                local
                    .game(game_key)
                    .unwrap()
                    .versions
                    .iter()
                    .map(|v| v.created_at)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
            );
            while local
                .game(game_key)
                .unwrap()
                .get(&version_id(stamp, &self.device_id, &selected.hash))
                .is_some()
            {
                stamp = stamp.checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "version timestamp overflow")
                })?;
            }
            let resolution = Version {
                id: version_id(stamp, &self.device_id, &selected.hash),
                parent: Some(version_id_.to_string()),
                merged_parents: heads.into_iter().filter(|id| id != version_id_).collect(),
                created_at: stamp,
                device: self.device_id.clone(),
                hash: selected.hash.clone(),
            };
            self.cache_snapshot(
                game_key,
                &resolution.id,
                &self.backend.get_snapshot(game_key, version_id_)?,
            )?;
            let id = resolution.id.clone();
            local.add_version(game_key, resolution);
            self.publish_local(&local)?;
            id
        } else {
            version_id_.to_string()
        };
        self.save_local(&local)?;
        self.mark_applied(game_key, &applied)?;

        Ok(PullOutcome::Applied {
            version: applied,
            skipped,
        })
    }
}
