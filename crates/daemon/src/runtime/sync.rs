use super::*;
impl Runtime {
    /// Ручная синхронизация: push локальных изменений, затем pull
    /// удалённых. Конфликт сообщается наружу, не разрешается молча.
    pub fn sync(&mut self, game_key: Option<&str>) -> ResponsePayload {
        self.last_scan = scanner::scan(&self.manifests, self.platform, &self.roots);

        let targets: Vec<&FoundGame> = match game_key {
            Some(key) => self
                .last_scan
                .iter()
                .filter(|g| g.game_key == key)
                .collect(),
            None => self.last_scan.iter().collect(),
        };
        if targets.is_empty() {
            return ResponsePayload::Sync(SyncPayload {
                results: Vec::new(),
            });
        }

        let engines = match self.engines() {
            Ok(e) => e,
            Err(msg) => {
                return ResponsePayload::Sync(SyncPayload {
                    results: targets
                        .iter()
                        .map(|g| SyncResult {
                            game_key: g.game_key.clone(),
                            outcome: "error".into(),
                            detail: Some(msg.clone()),
                        })
                        .collect(),
                })
            }
        };

        let mut results = Vec::new();
        for engine in &engines {
            for game in &targets {
                results.push(sync_one(
                    engine,
                    &self.manifests,
                    game,
                    self.platform,
                    &self.roots,
                ));
            }
        }
        ResponsePayload::Sync(SyncPayload { results })
    }
}
/// Префикс канала в detail: при нескольких бэкендах результаты игры
/// идут подряд, и без канала их не различить.
fn with_channel(channel: &str, mut r: SyncResult) -> SyncResult {
    if let Some(d) = r.detail {
        r.detail = Some(format!("[{channel}] {d}"));
    }
    r
}

fn sync_one(
    engine: &Engine<Box<dyn Backend>>,
    db: &ManifestDb,
    game: &FoundGame,
    platform: Platform,
    roots: &Roots,
) -> SyncResult {
    let channel = engine.backend_name();
    let manifest: &Manifest = match db.get(&game.game_key) {
        Some(m) => m,
        None => {
            return SyncResult {
                game_key: game.game_key.clone(),
                outcome: "skipped".into(),
                detail: Some(format!("[{channel}] нет манифеста")),
            }
        }
    };

    // Устройство, никогда не синхронизировавшее игру, принимает удалённую
    // линию версий целиком: push своего состояния как нового корня был бы
    // ложным ветвлением. Локальные сейвы сохраняются авто-бэкапом при pull.
    match (
        engine.has_local_versions(&game.game_key),
        engine.has_remote_versions(&game.game_key),
    ) {
        (Ok(false), Ok(true)) => {
            return with_channel(
                channel,
                match engine.pull(manifest, roots, platform) {
                    Ok(PullOutcome::Applied { version, .. }) => SyncResult {
                        game_key: game.game_key.clone(),
                        outcome: "applied".into(),
                        detail: Some(version),
                    },
                    Ok(PullOutcome::UpToDate) => SyncResult {
                        game_key: game.game_key.clone(),
                        outcome: "up_to_date".into(),
                        detail: None,
                    },
                    Ok(PullOutcome::Conflict(heads)) => SyncResult {
                        game_key: game.game_key.clone(),
                        outcome: "conflict".into(),
                        detail: Some(heads.join(", ")),
                    },
                    Ok(PullOutcome::NoRemoteVersion) => unreachable!("remote checked above"),
                    Ok(PullOutcome::Locked(holder)) => SyncResult {
                        game_key: game.game_key.clone(),
                        outcome: "locked".into(),
                        detail: Some(holder),
                    },
                    Err(e) => SyncResult {
                        game_key: game.game_key.clone(),
                        outcome: "error".into(),
                        detail: Some(e.to_string()),
                    },
                },
            );
        }
        (Err(e), _) => {
            return with_channel(
                channel,
                SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "error".into(),
                    detail: Some(e.to_string()),
                },
            );
        }
        _ => {}
    }

    let push = engine.push(&game.game_key, &game.locations);
    match &push {
        Err(e) => {
            return with_channel(
                channel,
                SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "error".into(),
                    detail: Some(e.to_string()),
                },
            )
        }
        Ok(PushOutcome::Locked(holder)) => {
            return with_channel(
                channel,
                SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "locked".into(),
                    detail: Some(holder.clone()),
                },
            )
        }
        _ => (),
    }
    let pull = engine.pull(manifest, roots, platform);

    // Порядок важен: конфликт и ошибки — раньше успешных исходов.
    let (outcome, detail) = match (push, pull) {
        (_, Ok(PullOutcome::Conflict(heads))) => ("conflict", Some(heads.join(", "))),
        (Err(e), _) => ("error", Some(e.to_string())),
        (_, Err(e)) => ("error", Some(e.to_string())),
        (Ok(PushOutcome::Locked(holder)), _) => ("locked", Some(holder)),
        (_, Ok(PullOutcome::Locked(holder))) => ("locked", Some(holder)),
        (Ok(PushOutcome::Uploaded(v)), _) => ("uploaded", Some(v.id)),
        (Ok(PushOutcome::Unchanged), Ok(PullOutcome::Applied { version, .. })) => {
            ("applied", Some(version))
        }
        (Ok(PushOutcome::Unchanged), Ok(PullOutcome::UpToDate)) => ("up_to_date", None),
        (Ok(PushOutcome::Unchanged), Ok(PullOutcome::NoRemoteVersion)) => ("up_to_date", None),
    };
    with_channel(
        channel,
        SyncResult {
            game_key: game.game_key.clone(),
            outcome: outcome.into(),
            detail,
        },
    )
}
