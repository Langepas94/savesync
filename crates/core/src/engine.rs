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
use crate::paths::{Platform, Roots};
use crate::scanner::SaveLocation;
use crate::snapshot;
use crate::snapshot::now_millis;

mod destinations;

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
}

fn sanitize(path: &Path) -> PathBuf {
    let s = path.to_string_lossy().replace([':', '\\'], "/");
    PathBuf::from(s.trim_start_matches('/').replace('/', "__"))
}

mod import;
mod locks;
mod restore;
mod state;
#[cfg(test)]
mod tests;
mod upload;
