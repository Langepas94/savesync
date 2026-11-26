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
    /// Other heads explicitly merged when the user resolves a conflict.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_parents: Vec<String>,
    pub created_at: u64,
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    pub hash: String,
}

impl Version {
    /// The optional display name can be absent when a 1.x client republishes a record.
    pub(crate) fn same_record(&self, other: &Self) -> bool {
        self.id == other.id
            && self.parent == other.parent
            && self.merged_parents == other.merged_parents
            && self.created_at == other.created_at
            && self.device == other.device
            && self.hash == other.hash
    }
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
            .flat_map(|v| {
                v.parent
                    .iter()
                    .chain(v.merged_parents.iter())
                    .map(String::as_str)
            })
            .collect();
        self.versions
            .iter()
            .filter(|v| !parents.contains(v.id.as_str()))
            .collect()
    }

    /// Единственная голова (нет конфликта).
    pub fn head(&self) -> Option<&Version> {
        let heads = self.heads();
        (heads.len() == 1).then(|| heads[0])
    }

    pub fn get(&self, id: &str) -> Option<&Version> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// Является ли `ancestor` предком `descendant` по цепочке parent.
    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let mut cur = self.get(descendant);
        let mut visited = std::collections::HashSet::new();
        while let Some(v) = cur {
            if !visited.insert(&v.id) {
                return false;
            }
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
        let duplicate = game.versions.iter().any(|e| {
            e.id == v.id
                || (e.hash == v.hash
                    && e.parent == v.parent
                    && e.merged_parents == v.merged_parents)
        });
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
                if let Some(existing) = game
                    .versions
                    .iter_mut()
                    .find(|existing| existing.id == v.id)
                {
                    if existing.device_name.is_none() && existing.same_record(v) {
                        existing.device_name = v.device_name.clone();
                    }
                } else {
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
#[path = "../tests/sync/index.rs"]
mod tests;
