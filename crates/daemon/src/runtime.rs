//! Ядро демона без транспорта: конфиг + манифесты + сканер + engine.
//! Покрыто тестами напрямую; IPC-сервер — тонкая обёртка над этим.

use std::io;
use std::path::PathBuf;

use savesync_core::backend::{Backend, FsBackend};
use savesync_core::config::Config;
use savesync_core::engine::{Engine, PullOutcome, PushOutcome};
use savesync_core::gdrive::GoogleDrive;
use savesync_core::ipc::{
    GameStatus, ResponsePayload, ScanPayload, ScannedGame, StatusPayload, SyncPayload, SyncResult,
};
use savesync_core::manifest::{Manifest, ManifestDb};
use savesync_core::paths::{Platform, Roots};
use savesync_core::scanner::{self, FoundGame};
use savesync_core::yandex::YandexDisk;

pub struct Runtime {
    pub config: Config,
    pub manifests: ManifestDb,
    pub platform: Platform,
    pub roots: Roots,
    /// Локальное состояние движка (индекс, tmp, бэкапы).
    pub state_dir: PathBuf,
    pub last_scan: Vec<FoundGame>,
}

impl Runtime {
    /// Собирает runtime из конфига. Манифесты загружаются из
    /// `config.manifests_dir` (может быть пустой базой) плюс custom-правила
    /// пользователя из `<state_dir>/custom/*.yaml` — они никогда не
    /// перезаписываются обновлением базы.
    pub fn new(config: Config, state_dir: PathBuf) -> io::Result<Self> {
        let mut manifests = match &config.manifests_dir {
            Some(dir) => {
                let (db, errors) = savesync_core::manifest::load_manifest_tree(dir);
                for e in &errors {
                    eprintln!("manifest load warning: {e}");
                }
                db
            }
            None => ManifestDb::default(),
        };
        let custom_dir = state_dir.join("custom");
        if custom_dir.is_dir() {
            let (custom_db, errors) = savesync_core::manifest::load_manifest_tree(&custom_dir);
            for e in &errors {
                eprintln!("custom manifest load warning: {e}");
            }
            for m in custom_db.iter() {
                manifests.insert(m.clone());
            }
        }
        let platform = Platform::current();
        let roots = Roots::detect(platform);
        Ok(Runtime {
            config,
            manifests,
            platform,
            roots,
            state_dir,
            last_scan: Vec::new(),
        })
    }

    /// Каналы синхронизации (plan §3): настроенные бэкенды, каждый —
    /// отдельный Engine. Каналы независимы: sync идёт по всем.
    fn engines(&self) -> Result<Vec<Engine<Box<dyn Backend>>>, String> {
        let mut out: Vec<Engine<Box<dyn Backend>>> = Vec::new();
        if let Some(fs) = &self.config.backends.fs_path {
            out.push(Engine::new(
                Box::new(FsBackend::new(fs)) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(ya) = &self.config.backends.yandex {
            out.push(Engine::new(
                Box::new(YandexDisk::new(&ya.token)) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(gg) = &self.config.backends.google {
            out.push(Engine::new(
                Box::new(GoogleDrive::new(&gg.token)) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if out.is_empty() {
            return Err(
                "канал синхронизации не настроен (backends.fs_path / yandex / google)".into(),
            );
        }
        Ok(out)
    }

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
        let games = self
            .last_scan
            .iter()
            .map(|g| GameStatus {
                game_key: g.game_key.clone(),
                display_name: g.display_name.clone(),
                // Подсчёт версий/состояние — TODO M2 (нужен публичный
                // доступ engine к индексу).
                versions: 0,
                state: "local".into(),
            })
            .collect();
        ResponsePayload::Status(StatusPayload {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            device_id: self.config.device_id.clone(),
            device_name: self.config.device_name.clone(),
            games,
        })
    }

    /// Ручная синхронизация: push локальных изменений, затем pull
    /// удалённых. Конфликт сообщается наружу, не разрешается молча.
    pub fn sync(&mut self, game_key: Option<&str>) -> ResponsePayload {
        if self.last_scan.is_empty() {
            self.last_scan = scanner::scan(&self.manifests, self.platform, &self.roots);
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use savesync_core::config::BackendsConfig;
    use savesync_core::manifest::parse_manifest_str;

    struct World {
        dir: PathBuf,
    }

    impl World {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "savesync-test-daemon-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            World { dir }
        }

        /// Симулирует «устройство»: своя home-папка с сейвом и state_dir.
        fn device(&self, name: &str) -> Runtime {
            let home = self.dir.join(name).join("home");
            std::fs::create_dir_all(home.join("game/saves")).unwrap();
            std::fs::write(home.join("game/saves/slot1.sav"), format!("save of {name}")).unwrap();

            let manifest = parse_manifest_str(
                "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
            )
            .unwrap();
            Runtime {
                config: Config {
                    version: 1,
                    device_id: name.into(),
                    device_name: name.into(),
                    manifests_dir: None,
                    backends: BackendsConfig {
                        fs_path: Some(self.dir.join("remote")),
                        ..BackendsConfig::default()
                    },
                },
                manifests: ManifestDb::from_entries(vec![manifest]),
                platform: Platform::Windows,
                roots: Roots {
                    home: Some(home),
                    ..Roots::default()
                },
                state_dir: self.dir.join(name).join("state"),
                last_scan: Vec::new(),
            }
        }

        fn save_of(&self, device: &str) -> String {
            std::fs::read_to_string(self.dir.join(device).join("home/game/saves/slot1.sav"))
                .unwrap()
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn sync_results(payload: ResponsePayload) -> Vec<SyncResult> {
        match payload {
            ResponsePayload::Sync(p) => p.results,
            _ => panic!("expected sync payload"),
        }
    }

    #[test]
    fn scan_finds_games_from_manifests() {
        let world = World::new("scan");
        let mut rt = world.device("pc");
        match rt.scan() {
            ResponsePayload::Scan(p) => {
                assert_eq!(p.found, 1);
                assert_eq!(p.games[0].game_key, "test-game");
            }
            _ => panic!("expected scan payload"),
        }
    }

    #[test]
    fn sync_uploads_then_second_device_applies() {
        let world = World::new("sync");
        let mut pc = world.device("pc");
        let mut deck = world.device("deck");

        let results = sync_results(pc.sync(None));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, "uploaded");

        let results = sync_results(deck.sync(None));
        assert_eq!(results[0].outcome, "applied");
        assert_eq!(world.save_of("deck"), "save of pc");

        // Повторный sync — ничего не изменилось.
        let results = sync_results(deck.sync(None));
        assert_eq!(results[0].outcome, "up_to_date");
    }

    #[test]
    fn sync_reports_conflict_without_resolving() {
        let world = World::new("conflict");
        let mut pc = world.device("pc");
        let mut deck = world.device("deck");

        pc.sync(None);
        deck.sync(None); // deck получил базу

        // Обе стороны расходятся от одной базы.
        std::fs::write(world.dir.join("pc/home/game/saves/slot1.sav"), "pc branch").unwrap();
        pc.sync(None);
        std::fs::write(
            world.dir.join("deck/home/game/saves/slot1.sav"),
            "deck branch",
        )
        .unwrap();
        deck.sync(None); // зальёт свою ветку

        let results = sync_results(deck.sync(Some("test-game")));
        assert_eq!(results[0].outcome, "conflict");
        // Конфликт не разрешён молча: локальный сейв на месте.
        assert_eq!(world.save_of("deck"), "deck branch");
    }

    #[test]
    fn sync_unknown_game_returns_empty() {
        let world = World::new("unknown");
        let mut pc = world.device("pc");
        assert!(sync_results(pc.sync(Some("no-such-game"))).is_empty());
    }

    #[test]
    fn custom_rules_from_state_dir_are_scanned() {
        let world = World::new("custom");
        let home = world.dir.join("pc/home");
        std::fs::create_dir_all(home.join("game/saves")).unwrap();
        std::fs::write(home.join("game/saves/slot1.sav"), "x").unwrap();

        // Custom-правило лежит в <state>/custom/<slug>.yaml.
        let state = world.dir.join("pc/state");
        let custom = state.join("custom");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(
            custom.join("my-game.yaml"),
            "schema: 1\nname: My Game\nslug: custom:my-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
        )
        .unwrap();

        let config = Config {
            version: 1,
            device_id: "pc".into(),
            device_name: "pc".into(),
            manifests_dir: None,
            backends: BackendsConfig::default(),
        };
        let mut rt = Runtime::new(config, state).unwrap();
        // Runtime::new детектит корни реальной машины — в тесте подменяем.
        rt.platform = Platform::Windows;
        rt.roots = Roots {
            home: Some(home),
            ..Roots::default()
        };

        match rt.scan() {
            ResponsePayload::Scan(p) => {
                assert_eq!(p.found, 1);
                assert_eq!(p.games[0].game_key, "custom:my-game");
                assert_eq!(p.games[0].display_name, "My Game");
            }
            _ => panic!("expected scan payload"),
        }
    }
}
