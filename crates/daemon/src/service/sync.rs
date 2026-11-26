use super::Runtime;
use savesync_core::backends::backend::Backend;
use savesync_core::catalog::manifest::{Manifest, ManifestDb};
use savesync_core::catalog::scanner::FoundGame;
use savesync_core::ipc::{SyncPayload, SyncResult};
use savesync_core::platform::paths::{Platform, Roots};
use savesync_core::sync::engine::{Engine, PullOutcome, PushOutcome};
impl Runtime {
    /// Ручная синхронизация: push локальных изменений, затем pull
    /// удалённых. Конфликт сообщается наружу, не разрешается молча.
    pub fn sync(&mut self, game_key: Option<&str>) -> SyncPayload {
        self.refresh_catalog();

        let targets: Vec<&FoundGame> = match game_key {
            Some(key) => self
                .last_scan
                .iter()
                .filter(|g| g.game_key == key)
                .collect(),
            None => self.last_scan.iter().collect(),
        };
        if targets.is_empty() {
            return SyncPayload {
                results: Vec::new(),
            };
        }

        let engines = match self.engines() {
            Ok(e) => e,
            Err(msg) => {
                return SyncPayload {
                    results: targets
                        .iter()
                        .map(|g| SyncResult {
                            game_key: g.game_key.clone(),
                            outcome: "error".into(),
                            detail: Some(msg.clone()),
                        })
                        .collect(),
                }
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
        if results.iter().any(|result| result.outcome == "applied") {
            self.refresh_catalog();
        }
        SyncPayload { results }
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

    if manifest.portability.status == savesync_core::catalog::manifest::PortabilityStatus::Blocked {
        return with_channel(
            channel,
            SyncResult {
                game_key: game.game_key.clone(),
                outcome: "skipped".into(),
                detail: Some(
                    manifest
                        .portability
                        .reason
                        .clone()
                        .unwrap_or_else(|| "синхронизация сейвов заблокирована".into()),
                ),
            },
        );
    }

    if game.locations.is_empty() {
        return with_channel(
            channel,
            match engine.pull(manifest, roots, platform) {
                Ok(PullOutcome::Applied { version, .. }) => SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "applied".into(),
                    detail: Some(version),
                },
                Ok(PullOutcome::NoRemoteVersion) => SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "no_saves".into(),
                    detail: Some("сейвы ещё не созданы".into()),
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
                Ok(PullOutcome::Locked(holder)) => SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "locked".into(),
                    detail: Some(holder),
                },
                Err(error) => SyncResult {
                    game_key: game.game_key.clone(),
                    outcome: "error".into(),
                    detail: Some(error.to_string()),
                },
            },
        );
    }

    // Preserve independent local saves as a separate branch on first sync.
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
