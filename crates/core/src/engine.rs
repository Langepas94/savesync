//! Sync engine M1: ручная синхронизация снапшотами через бэкенд.
//! Без таймеров, watcher'ов и автотриггеров (итерация 2).
//!
//! Потоки:
//! - `push` — снапшот локальных сейвов → бэкенд, если контент изменился;
//! - `pull` — merge индекса → конфликт-политика → авто-бэкап → распаковка.
//!
//! Перед любым затиранием локальных сейвов делается авто-бэкап движка
//! (`<state>/backups/<game>/<ts>/`), retention — M6.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::backend::Backend;
use crate::index::{version_id, ConflictPolicy, Index, Version};
use crate::manifest::Manifest;
use crate::paths::{resolve_pattern_multi, Platform, Roots};
use crate::scanner::SaveLocation;
use crate::snapshot;
use crate::snapshot::now_millis;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("snapshot corrupted: {0}")]
    CorruptedSnapshot(String),
    #[error("remote index is invalid: {0}")]
    BadRemoteIndex(serde_json::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// Контент не изменился — новая версия не создана.
    Unchanged,
    Uploaded(Version),
    /// Игра залочена другим устройством (свежий heartbeat).
    Locked(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum PullOutcome {
    /// Локально уже применена актуальная версия.
    UpToDate,
    /// Версия применена; skipped — паттерны, не резолвящиеся локально.
    Applied {
        version: String,
        skipped: Vec<String>,
    },
    /// Ветвление: несколько голов, политика Ask — решение за пользователем.
    Conflict(Vec<String>),
    /// На бэкенде нет версий этой игры.
    NoRemoteVersion,
    /// Игра залочена другим устройством (свежий heartbeat).
    Locked(String),
}

/// Результат импорта файла бэкапа (plan §2.2).
#[derive(Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    /// Зарегистрирована новая версия в линии игры.
    Registered(Version),
    /// Такой контент уже есть от той же базы — версия не создана.
    Duplicate(String),
}

/// Движок синхронизации одного устройства.
pub struct Engine<B: Backend> {
    backend: B,
    device_id: String,
    /// Локальное состояние: `index.json` (с `applied`), `tmp/`, `backups/`.
    state_dir: PathBuf,
    /// Сколько авто-бэкапов на игру хранить (plan §2.1, retention).
    backup_retention: u32,
}

impl<B: Backend> Engine<B> {
    pub fn new(backend: B, device_id: impl Into<String>, state_dir: impl Into<PathBuf>) -> Self {
        Engine {
            backend,
            device_id: device_id.into(),
            state_dir: state_dir.into(),
            backup_retention: 10,
        }
    }

    /// Retention авто-бэкапов перед затиранием (по умолчанию 10).
    pub fn with_backup_retention(mut self, keep: u32) -> Self {
        self.backup_retention = keep;
        self
    }

    /// Имя канала синхронизации (для логов и отчётов sync).
    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    fn local_index_path(&self) -> PathBuf {
        self.state_dir.join("index.json")
    }

    /// Локальное (per-device) состояние: какая версия применена на этом
    /// устройстве. Хранится отдельно от индекса, чтобы не протекать
    /// в общий index.json бэкенда.
    fn applied_path(&self) -> PathBuf {
        self.state_dir.join("applied.json")
    }

    fn load_applied(&self) -> io::Result<std::collections::BTreeMap<String, String>> {
        match std::fs::read(self.applied_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(e),
        }
    }

    fn mark_applied(&self, game_key: &str, version: &str) -> io::Result<()> {
        let mut applied = self.load_applied()?;
        applied.insert(game_key.to_string(), version.to_string());
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::write(
            self.applied_path(),
            serde_json::to_vec_pretty(&applied)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        )
    }

    fn applied_of(&self, game_key: &str) -> Option<String> {
        self.load_applied().ok()?.get(game_key).cloned()
    }

    fn load_local(&self) -> io::Result<Index> {
        Index::load(&self.local_index_path())
    }

    fn save_local(&self, index: &Index) -> io::Result<()> {
        std::fs::create_dir_all(&self.state_dir)?;
        index.save(&self.local_index_path())
    }

    fn load_remote(&self) -> Result<Index, EngineError> {
        match self.backend.get_index()? {
            None => Ok(Index::new()),
            Some(bytes) => serde_json::from_slice(&bytes).map_err(EngineError::BadRemoteIndex),
        }
    }

    fn save_remote(&self, index: &Index) -> Result<(), EngineError> {
        let bytes = serde_json::to_vec(index).map_err(EngineError::BadRemoteIndex)?;
        self.backend.put_index(&bytes)?;
        Ok(())
    }

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

    /// Выгрузка: снапшот → бэкенд → обновление локального и общего индексов.
    pub fn push(
        &self,
        game_key: &str,
        locations: &[SaveLocation],
    ) -> Result<PushOutcome, EngineError> {
        // Игра запущена на другом устройстве — ждём (plan §2.1).
        if let Some(holder) = self.foreign_lock(game_key)? {
            return Ok(PushOutcome::Locked(holder));
        }

        let tmp_dir = self.state_dir.join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let tmp_snap = tmp_dir.join(format!("{game_key}.zip"));

        let manifest = snapshot::create(game_key, &self.device_id, locations, &tmp_snap)?;

        let mut local = self.load_local()?;
        let local_head = local.game(game_key).and_then(|g| g.head().cloned());
        if local_head.as_ref().map(|h| h.hash.as_str()) == Some(manifest.content_hash.as_str()) {
            std::fs::remove_file(&tmp_snap).ok();
            return Ok(PushOutcome::Unchanged);
        }

        let id = version_id(manifest.created_at, &self.device_id, &manifest.content_hash);
        let version = Version {
            parent: local_head.map(|h| h.id),
            id: id.clone(),
            created_at: manifest.created_at,
            device: self.device_id.clone(),
            hash: manifest.content_hash,
        };

        let data = std::fs::read(&tmp_snap)?;
        self.backend.put_snapshot(game_key, &id, &data)?;
        std::fs::remove_file(&tmp_snap).ok();

        local.add_version(game_key, version.clone());
        self.save_local(&local)?;
        self.mark_applied(game_key, &id)?;

        // Общий индекс: многосторонний merge по id версий.
        let mut remote = self.load_remote()?;
        remote.merge(&local);
        self.save_remote(&remote)?;

        Ok(PushOutcome::Uploaded(version))
    }

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
        local.merge(&remote);

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
        let mut local = self.load_local()?;

        if self.applied_of(game_key).as_deref() == Some(version_id_) {
            return Ok(PullOutcome::UpToDate);
        }
        if local
            .game(game_key)
            .and_then(|g| g.get(version_id_))
            .is_none()
        {
            // Версии нет в локальном индексе — подтянем индекс с бэкенда.
            let remote = self.load_remote()?;
            local.merge(&remote);
        }

        let bytes = self.backend.get_snapshot(game_key, version_id_)?;
        let tmp_dir = self.state_dir.join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let snap_path = tmp_dir.join(format!("{game_key}-{version_id_}.zip"));
        std::fs::write(&snap_path, bytes)?;

        if !snapshot::verify(&snap_path)? {
            std::fs::remove_file(&snap_path).ok();
            return Err(EngineError::CorruptedSnapshot(version_id_.into()));
        }
        let snap_manifest = snapshot::read_manifest(&snap_path)?;

        // Базы локаций на ЭТОМ устройстве: паттерн -> резолв -> база.
        let mut skipped = Vec::new();
        let mut targets: Vec<(&snapshot::LocationManifest, PathBuf)> = Vec::new();
        for loc in &snap_manifest.locations {
            match self.resolve_base(manifest, roots, platform, &loc.pattern) {
                Some(base) => targets.push((loc, base)),
                None => skipped.push(loc.pattern.clone()),
            }
        }

        // Авто-бэкап текущего состояния перед затиранием (plan §2.1).
        let stamp = snapshot::now_millis();
        for (_, base) in &targets {
            if base.exists() {
                let backup = self
                    .state_dir
                    .join("backups")
                    .join(game_key)
                    .join(stamp.to_string())
                    .join(sanitize(base));
                copy_dir(base, &backup)?;
            }
        }
        // Retention авто-бэкапов (plan §2.1/§2.2).
        let _ = crate::backup::prune_dir_backups(
            &self.state_dir,
            game_key,
            self.backup_retention as usize,
        );

        for (loc, base) in &targets {
            snapshot::extract_location(&snap_path, loc, base)?;
        }
        std::fs::remove_file(&snap_path).ok();

        self.save_local(&local)?;
        self.mark_applied(game_key, version_id_)?;

        Ok(PullOutcome::Applied {
            version: version_id_.to_string(),
            skipped,
        })
    }

    /// Импорт переносимого бэкапа `*.savesync-backup` (plan §2.2):
    /// файл регистрируется как новая версия в линии игры (видна в
    /// истории, можно откатиться через pull_version). Само применение
    /// сейвов — отдельным pull.
    pub fn import_backup(&self, path: &Path) -> Result<ImportOutcome, EngineError> {
        if !snapshot::verify(path)? {
            return Err(EngineError::CorruptedSnapshot(path.display().to_string()));
        }
        let snap_manifest = snapshot::read_manifest(path)?;
        let game_key = snap_manifest.game_key.clone();

        let mut local = self.load_local()?;
        let head = local.game(&game_key).and_then(|g| g.head().cloned());
        // Тот же контент от той же базы уже есть — дубликат.
        if head.as_ref().map(|h| h.hash.as_str()) == Some(snap_manifest.content_hash.as_str()) {
            return Ok(ImportOutcome::Duplicate(head.unwrap().id));
        }

        let id = version_id(now_millis(), &self.device_id, &snap_manifest.content_hash);
        let version = Version {
            parent: head.map(|h| h.id),
            id: id.clone(),
            created_at: snap_manifest.created_at,
            device: format!("{}(import)", snap_manifest.device_id),
            hash: snap_manifest.content_hash,
        };

        let data = std::fs::read(path)?;
        self.backend.put_snapshot(&game_key, &id, &data)?;

        local.add_version(&game_key, version.clone());
        self.save_local(&local)?;

        let mut remote = self.load_remote()?;
        remote.merge(&local);
        self.save_remote(&remote)?;

        Ok(ImportOutcome::Registered(version))
    }

    /// Резолвит базу локации по паттерну из манифеста игры.
    fn resolve_base(
        &self,
        manifest: &Manifest,
        roots: &Roots,
        platform: Platform,
        pattern: &str,
    ) -> Option<PathBuf> {
        // Паттерн обязан быть правилом этой игры — иначе не знаем,
        // к какой локации он относится на этом устройстве.
        manifest
            .paths_for(platform)
            .find(|r| r.pattern == pattern)?;
        let resolved = resolve_pattern_multi(pattern, roots);
        resolved.first().map(|r| snapshot::location_base(r))
    }
}

fn sanitize(path: &Path) -> PathBuf {
    let s = path.to_string_lossy().replace([':', '\\'], "/");
    PathBuf::from(s.trim_start_matches('/').replace('/', "__"))
}

fn copy_dir(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_dir(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest_str;
    use crate::scanner::SaveLocation;

    /// Симуляция «устройства»: engine + корни + домашняя папка.
    struct Device {
        engine: Engine<crate::backend::FsBackend>,
        home: PathBuf,
        roots: Roots,
    }

    struct World {
        dir: PathBuf,
    }

    impl World {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "savesync-test-engine-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            World { dir }
        }

        fn backend(&self) -> crate::backend::FsBackend {
            crate::backend::FsBackend::new(self.dir.join("remote"))
        }

        fn device(&self, name: &str) -> Device {
            let home = self.dir.join(name).join("home");
            std::fs::create_dir_all(&home).unwrap();
            Device {
                engine: Engine::new(self.backend(), name, self.dir.join(name).join("state")),
                roots: Roots {
                    home: Some(home.clone()),
                    ..Roots::default()
                },
                home,
            }
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn manifest() -> Manifest {
        parse_manifest_str(
            "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
        )
        .unwrap()
    }

    impl Device {
        fn write_save(&self, content: &str) {
            let dir = self.home.join("game/saves");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("slot1.sav"), content).unwrap();
        }

        fn read_save(&self) -> String {
            String::from_utf8(std::fs::read(self.home.join("game/saves/slot1.sav")).unwrap())
                .unwrap()
        }

        fn locations(&self) -> Vec<SaveLocation> {
            let resolved = self
                .home
                .join("game/saves")
                .to_string_lossy()
                .replace('\\', "/");
            vec![SaveLocation {
                pattern: "<home>/game/saves".into(),
                resolved: resolved.clone(),
                hits: vec![PathBuf::from(resolved)],
            }]
        }
    }

    #[test]
    fn push_then_pull_to_second_device() {
        let world = World::new("two-devices");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        pc.write_save("pc progress");
        let outcome = pc.engine.push("test-game", &pc.locations()).unwrap();
        assert!(matches!(outcome, PushOutcome::Uploaded(_)));

        let outcome = deck
            .engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();
        assert!(matches!(outcome, PullOutcome::Applied { .. }));
        assert_eq!(deck.read_save(), "pc progress");

        // Повторный pull — уже актуально.
        assert_eq!(
            deck.engine
                .pull(&m, &deck.roots, Platform::Windows)
                .unwrap(),
            PullOutcome::UpToDate
        );
    }

    #[test]
    fn push_without_changes_is_unchanged() {
        let world = World::new("unchanged");
        let pc = world.device("pc");
        pc.write_save("v1");
        assert!(matches!(
            pc.engine.push("test-game", &pc.locations()).unwrap(),
            PushOutcome::Uploaded(_)
        ));
        assert_eq!(
            pc.engine.push("test-game", &pc.locations()).unwrap(),
            PushOutcome::Unchanged
        );
    }

    #[test]
    fn foreign_lock_blocks_push_and_pull() {
        let world = World::new("locked");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        // PC залочил игру (игра запущена на PC).
        assert!(pc.engine.lock_game("test-game").unwrap());
        assert_eq!(
            deck.engine.foreign_lock("test-game").unwrap().as_deref(),
            Some("pc")
        );

        // Deck не может ни выгрузить, ни подтянуть.
        deck.write_save("deck progress");
        assert_eq!(
            deck.engine.push("test-game", &deck.locations()).unwrap(),
            PushOutcome::Locked("pc".into())
        );
        assert_eq!(
            deck.engine
                .pull(&m, &deck.roots, Platform::Windows)
                .unwrap(),
            PullOutcome::Locked("pc".into())
        );

        // Свой лок push'у не мешает; повторный захват продлевает heartbeat.
        assert!(pc.engine.lock_game("test-game").unwrap());
        pc.write_save("pc progress");
        assert!(matches!(
            pc.engine.push("test-game", &pc.locations()).unwrap(),
            PushOutcome::Uploaded(_)
        ));

        // Снятие лока отпускает игру.
        pc.engine.unlock_game("test-game").unwrap();
        assert_eq!(deck.engine.foreign_lock("test-game").unwrap(), None);
        assert!(matches!(
            deck.engine
                .pull(&m, &deck.roots, Platform::Windows)
                .unwrap(),
            PullOutcome::Applied { .. }
        ));
        assert_eq!(deck.read_save(), "pc progress");
    }

    #[test]
    fn import_backup_registers_version() {
        let world = World::new("import");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        // PC делает переносимый бэкап (например, на флешку).
        pc.write_save("flash progress");
        let backup_dir = world.dir.join("usb");
        let file =
            crate::backup::create_backup("test-game", "pc", &pc.locations(), &backup_dir).unwrap();

        // deck импортирует его как новую версию в линии игры.
        let outcome = deck.engine.import_backup(&file).unwrap();
        let ImportOutcome::Registered(v) = outcome else {
            panic!("expected Registered, got {outcome:?}")
        };
        assert!(v.parent.is_none());
        assert_eq!(v.device, "pc(import)");

        // Версия видна в линии и применяется обычным pull.
        let pulled = deck
            .engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();
        assert!(matches!(pulled, PullOutcome::Applied { .. }));
        assert_eq!(deck.read_save(), "flash progress");

        // Повторный импорт того же файла — дубликат, версия не плодится.
        let again = deck.engine.import_backup(&file).unwrap();
        assert!(matches!(again, ImportOutcome::Duplicate(_)));
    }

    #[test]
    fn two_way_sync_roundtrip() {
        let world = World::new("roundtrip");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        pc.write_save("from pc");
        pc.engine.push("test-game", &pc.locations()).unwrap();
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();

        deck.write_save("deck progress");
        deck.engine.push("test-game", &deck.locations()).unwrap();
        let outcome = pc.engine.pull(&m, &pc.roots, Platform::Windows).unwrap();
        assert!(matches!(outcome, PullOutcome::Applied { .. }));
        assert_eq!(pc.read_save(), "deck progress");
    }

    #[test]
    fn diverged_pushes_conflict_under_ask() {
        let world = World::new("conflict");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        pc.write_save("base");
        pc.engine.push("test-game", &pc.locations()).unwrap();
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();

        // Обе стороны делают снапшот от одной базы — ветвление.
        pc.write_save("pc branch");
        pc.engine.push("test-game", &pc.locations()).unwrap();
        deck.write_save("deck branch");
        deck.engine.push("test-game", &deck.locations()).unwrap();

        let outcome = deck
            .engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();
        match outcome {
            PullOutcome::Conflict(heads) => assert_eq!(heads.len(), 2),
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[test]
    fn latest_wins_policy_resolves_conflict() {
        let world = World::new("latest-wins");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        pc.write_save("base");
        pc.engine.push("test-game", &pc.locations()).unwrap();
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();

        pc.write_save("older branch");
        pc.engine.push("test-game", &pc.locations()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2)); // разные created_at
        deck.write_save("newer branch");
        deck.engine.push("test-game", &deck.locations()).unwrap();

        // Переключаем политику на auto-latest в локальном индексе pc
        // (на deck LatestWins выбрал бы его же собственную голову —
        // для него состояние уже актуально).
        let mut idx = pc.engine.load_local().unwrap();
        idx.game_mut("test-game").policy = ConflictPolicy::LatestWins;
        pc.engine.save_local(&idx).unwrap();

        let outcome = pc.engine.pull(&m, &pc.roots, Platform::Windows).unwrap();
        assert!(matches!(outcome, PullOutcome::Applied { .. }));
        assert_eq!(pc.read_save(), "newer branch");
        // Проигравшая ветка не удалена — авто-выбор обратим.
        let idx = pc.engine.load_local().unwrap();
        assert_eq!(idx.game("test-game").unwrap().versions.len(), 3);
    }

    #[test]
    fn pull_backs_up_before_overwrite() {
        let world = World::new("backup");
        let pc = world.device("pc");
        let deck = world.device("deck");
        let m = manifest();

        pc.write_save("remote version");
        pc.engine.push("test-game", &pc.locations()).unwrap();

        deck.write_save("local precious");
        deck.engine
            .pull(&m, &deck.roots, Platform::Windows)
            .unwrap();
        assert_eq!(deck.read_save(), "remote version");

        // Локальное состояние сохранено в авто-бэкапе движка.
        let backups = deck.engine.state_dir.join("backups").join("test-game");
        let entries: Vec<_> = std::fs::read_dir(&backups).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let backup_dir = entries[0].as_ref().unwrap().path();
        let found = find_file(&backup_dir, "slot1.sav").expect("backup must contain slot1.sav");
        assert_eq!(std::fs::read_to_string(found).unwrap(), "local precious");
    }

    fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()? {
            let path = entry.ok()?.path();
            if path.is_dir() {
                if let Some(found) = find_file(&path, name) {
                    return Some(found);
                }
            } else if path.file_name()?.to_str()? == name {
                return Some(path);
            }
        }
        None
    }

    #[test]
    fn rollback_to_specific_version() {
        let world = World::new("rollback");
        let pc = world.device("pc");
        let m = manifest();

        pc.write_save("v1 content");
        pc.engine.push("test-game", &pc.locations()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        pc.write_save("v2 content");
        let v2 = match pc.engine.push("test-game", &pc.locations()).unwrap() {
            PushOutcome::Uploaded(v) => v,
            _ => panic!("expected upload"),
        };

        let idx = pc.engine.load_local().unwrap();
        let v1 = idx
            .game("test-game")
            .unwrap()
            .versions
            .iter()
            .find(|v| v.id != v2.id)
            .unwrap()
            .id
            .clone();

        let outcome = pc
            .engine
            .pull_version(&m, &pc.roots, Platform::Windows, &v1)
            .unwrap();
        assert!(matches!(outcome, PullOutcome::Applied { .. }));
        assert_eq!(pc.read_save(), "v1 content");
    }
}
