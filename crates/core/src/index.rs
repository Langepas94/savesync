//! Локальный индекс версий (`index.json`): реестр игр, устройств и
//! иммутабельных версий снапшотов. Merge индексов с бэкенда — объединение
//! по id версий; конфликт = ветвление (несколько детей одного parent).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Политика разрешения конфликтов per-game (plan §2.1). Любой авто-выбор
/// обратим: проигравшая ветка остаётся в индексе.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPolicy {
    /// Спрашивать пользователя (дефолт для игр с длинными кампаниями).
    #[default]
    Ask,
    /// Авто: последняя по времени версия.
    LatestWins,
    /// Keep both: проигравшая ветка сохраняется боковой версией.
    KeepBoth,
}

/// Лок игры (plan §2.1): пока игра запущена на устройстве, её game_key
/// залочен в общем индексе; другие устройства не синхронизируют её.
/// Лок снимается явно либо протухает по heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameLock {
    pub device: String,
    /// Последний heartbeat, unix millis.
    pub heartbeat: u64,
}

/// Лок без heartbeat дольше этого интервала считается протухшим
/// (устройство выключилось, не сняв лок).
pub const LOCK_STALE_MS: u64 = 120_000;

/// Одна версия снапшота в линии игры.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// `v<created_at>_<device>_<hash8>` — иммутабельный id, он же имя файла
    /// снапшота на бэкенде (`<id>.zip`).
    pub id: String,
    /// Версия, от которой сделан снапшот (None — первая).
    pub parent: Option<String>,
    pub created_at: u64,
    pub device: String,
    pub hash: String,
}

/// Индекс одной игры.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameIndex {
    #[serde(default)]
    pub policy: ConflictPolicy,
    #[serde(default)]
    pub versions: Vec<Version>,
}

impl GameIndex {
    /// «Головы» линии: версии, не являющиеся чьим-либо parent.
    /// Больше одной головы = ветвление = конфликт.
    pub fn heads(&self) -> Vec<&Version> {
        let parents: std::collections::HashSet<&str> = self
            .versions
            .iter()
            .filter_map(|v| v.parent.as_deref())
            .collect();
        self.versions
            .iter()
            .filter(|v| !parents.contains(v.id.as_str()))
            .collect()
    }

    /// Единственная голова (нет конфликта).
    pub fn head(&self) -> Option<&Version> {
        let heads = self.heads();
        (heads.len() == 1).then_some(heads[0])
    }

    pub fn get(&self, id: &str) -> Option<&Version> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// Является ли `ancestor` предком `descendant` по цепочке parent.
    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let mut cur = self.get(descendant);
        while let Some(v) = cur {
            if v.id == ancestor {
                return true;
            }
            cur = v.parent.as_deref().and_then(|p| self.get(p));
        }
        false
    }
}

/// Корневой индекс.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    #[serde(default)]
    pub games: BTreeMap<String, GameIndex>,
    /// Локи игр: game_key -> держатель. Живут в общем индексе бэкенда.
    #[serde(default)]
    pub locks: BTreeMap<String, GameLock>,
}

impl Index {
    pub fn new() -> Self {
        Index {
            version: 1,
            games: BTreeMap::new(),
            locks: BTreeMap::new(),
        }
    }

    pub fn game(&self, game_key: &str) -> Option<&GameIndex> {
        self.games.get(game_key)
    }

    pub fn game_mut(&mut self, game_key: &str) -> &mut GameIndex {
        self.games.entry(game_key.to_string()).or_default()
    }

    /// Захват/продление лока (heartbeat = now). Перезахват чужого
    /// свежего лока запрещён — возвращает false.
    pub fn acquire_lock(&mut self, game_key: &str, device: &str, now: u64) -> bool {
        if let Some(holder) = self.lock_holder(game_key, now) {
            if holder.device != device {
                return false;
            }
        }
        self.locks.insert(
            game_key.to_string(),
            GameLock {
                device: device.to_string(),
                heartbeat: now,
            },
        );
        true
    }

    /// Снятие лока; чужой лок не трогаем.
    pub fn release_lock(&mut self, game_key: &str, device: &str) {
        if self.locks.get(game_key).map(|l| l.device.as_str()) == Some(device) {
            self.locks.remove(game_key);
        }
    }

    /// Держатель свежего лока (протухшие игнорируются).
    pub fn lock_holder(&self, game_key: &str, now: u64) -> Option<&GameLock> {
        self.locks
            .get(game_key)
            .filter(|l| now.saturating_sub(l.heartbeat) < LOCK_STALE_MS)
    }

    /// Добавляет версию; дедупликация по id и по hash+parent
    /// (одинаковый контент от той же базы не плодит версии).
    /// Возвращает true, если версия реально добавлена.
    pub fn add_version(&mut self, game_key: &str, v: Version) -> bool {
        let game = self.game_mut(game_key);
        let duplicate = game
            .versions
            .iter()
            .any(|e| e.id == v.id || (e.hash == v.hash && e.parent == v.parent));
        if duplicate {
            return false;
        }
        game.versions.push(v);
        true
    }

    /// Многосторонний merge: объединение версий по id. При равном id
    /// записи иммутабельны — расхождений быть не должно; побеждает
    /// первая увиденная.
    pub fn merge(&mut self, other: &Index) {
        for (game_key, other_game) in &other.games {
            let game = self.game_mut(game_key);
            for v in &other_game.versions {
                if game.get(&v.id).is_none() {
                    game.versions.push(v.clone());
                }
            }
        }
        // Локи: побеждает свежайший heartbeat. Снятие лока через merge
        // не распространяется — полагаемся на протухание (LOCK_STALE_MS).
        for (game_key, other_lock) in &other.locks {
            match self.locks.get(game_key) {
                Some(own) if own.heartbeat >= other_lock.heartbeat => {}
                _ => {
                    self.locks.insert(game_key.clone(), other_lock.clone());
                }
            }
        }
    }

    pub fn load(path: &Path) -> io::Result<Index> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Index::new()),
            Err(e) => Err(e),
        }
    }

    /// Атомарная запись: tmp-файл + rename.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }
}

/// Id версии: `v<millis>_<device>_<hash8>`.
pub fn version_id(created_at: u64, device: &str, hash: &str) -> String {
    let short: String = hash.chars().take(8).collect();
    format!("v{created_at}_{device}_{short}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str, parent: Option<&str>, hash: &str) -> Version {
        Version {
            id: id.into(),
            parent: parent.map(Into::into),
            created_at: 1,
            device: "dev".into(),
            hash: hash.into(),
        }
    }

    #[test]
    fn linear_history_has_single_head() {
        let mut idx = Index::new();
        idx.add_version("g", v("v1", None, "a"));
        idx.add_version("g", v("v2", Some("v1"), "b"));
        let game = idx.game("g").unwrap();
        assert_eq!(game.head().unwrap().id, "v2");
        assert!(game.is_ancestor("v1", "v2"));
        assert!(!game.is_ancestor("v2", "v1"));
    }

    #[test]
    fn branching_is_detected_as_conflict() {
        let mut idx = Index::new();
        idx.add_version("g", v("v1", None, "a"));
        idx.add_version("g", v("v2-pc", Some("v1"), "b"));
        idx.add_version("g", v("v2-deck", Some("v1"), "c"));
        let heads = idx.game("g").unwrap().heads();
        assert_eq!(heads.len(), 2);
        assert!(idx.game("g").unwrap().head().is_none());
    }

    #[test]
    fn add_version_dedupes_by_hash_and_parent() {
        let mut idx = Index::new();
        assert!(idx.add_version("g", v("v1", None, "a")));
        // Тот же контент от той же базы — не версия (пустые изменения).
        assert!(!idx.add_version("g", v("v1-other-device", None, "a")));
        // Тот же контент от другой базы — новая ветка, сохраняем.
        assert!(idx.add_version("g", v("v2", Some("v1"), "a")));
        assert_eq!(idx.game("g").unwrap().versions.len(), 2);
    }

    #[test]
    fn merge_unions_versions_by_id() {
        let mut a = Index::new();
        a.add_version("g", v("v1", None, "a"));
        let mut b = Index::new();
        b.add_version("g", v("v1", None, "a"));
        b.add_version("g", v("v2", Some("v1"), "b"));

        a.merge(&b);
        assert_eq!(a.game("g").unwrap().versions.len(), 2);
        // Повторный merge идемпотентен.
        a.merge(&b);
        assert_eq!(a.game("g").unwrap().versions.len(), 2);
    }

    #[test]
    fn save_load_roundtrip_and_atomicity() {
        let dir = std::env::temp_dir().join(format!("savesync-test-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.json");

        let mut idx = Index::new();
        idx.add_version("elden-ring", v("v1", None, "abc"));
        idx.save(&path).unwrap();

        let loaded = Index::load(&path).unwrap();
        assert_eq!(loaded, idx);
        // Отсутствующий файл — пустой индекс, а не ошибка.
        assert_eq!(Index::load(&dir.join("nope.json")).unwrap(), Index::new());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn version_id_format() {
        let id = version_id(1_700_000_000_000, "deck", "abcdef0123456789");
        assert_eq!(id, "v1700000000000_deck_abcdef01");
    }

    #[test]
    fn lock_acquire_release_and_staleness() {
        let mut idx = Index::new();
        assert!(idx.acquire_lock("g", "pc", 1000));
        // Чужое устройство не перехватывает свежий лок.
        assert!(!idx.acquire_lock("g", "deck", 1001));
        // Своё устройство продлевает heartbeat.
        assert!(idx.acquire_lock("g", "pc", 2000));
        assert_eq!(idx.lock_holder("g", 3000).unwrap().device, "pc");
        // Чужой лок нельзя снять.
        idx.release_lock("g", "deck");
        assert!(idx.lock_holder("g", 3000).is_some());
        // Свой — можно.
        idx.release_lock("g", "pc");
        assert!(idx.lock_holder("g", 3000).is_none());
        // Протухший лок не держит игру.
        assert!(idx.acquire_lock("g", "pc", 10_000));
        assert!(idx.lock_holder("g", 10_000 + LOCK_STALE_MS).is_none());
        assert!(idx.acquire_lock("g", "deck", 10_000 + LOCK_STALE_MS));
    }

    #[test]
    fn merge_keeps_freshest_lock() {
        let mut a = Index::new();
        a.acquire_lock("g", "pc", 1000);
        let mut b = Index::new();
        b.acquire_lock("g", "deck", 2000);

        a.merge(&b);
        assert_eq!(a.locks["g"].device, "deck");
        assert_eq!(a.locks["g"].heartbeat, 2000);
        // Обратный merge не откатывает на более старый.
        b.merge(&a);
        assert_eq!(b.locks["g"].heartbeat, 2000);
    }

    #[test]
    fn locks_survive_save_load() {
        let dir = std::env::temp_dir().join(format!("savesync-test-locks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("index.json");

        let mut idx = Index::new();
        idx.acquire_lock("elden-ring", "pc", 42);
        idx.save(&path).unwrap();
        let loaded = Index::load(&path).unwrap();
        assert_eq!(loaded.locks["elden-ring"].heartbeat, 42);
        // Старый индекс без поля locks читается как пустой набор.
        let legacy = br#"{"version":1,"games":{}}"#;
        let parsed: Index = serde_json::from_slice(legacy).unwrap();
        assert!(parsed.locks.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
