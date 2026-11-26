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

use crate::backends::backend::Backend;
use crate::catalog::manifest::Manifest;
use crate::catalog::scanner::SaveLocation;
use crate::platform::paths::{Platform, Roots};
use crate::storage::snapshot;
use crate::storage::snapshot::now_millis;
use crate::sync::index::{version_id, ConflictPolicy, Index, Version};

mod destinations;
mod file_transaction;
mod restore_plan;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("snapshot corrupted: {0}")]
    CorruptedSnapshot(String),
    #[error("remote index is invalid: {0}")]
    BadRemoteIndex(serde_json::Error),
    #[error(
        "version {version} restored locally; remote publication pending, retry sync: {source}"
    )]
    RestorePublicationPending { version: String, source: io::Error },
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
    history_channel: Option<&'static str>,
    device_id: String,
    device_name: Option<String>,
    /// Локальное состояние: `index.json` (с `applied`), `tmp/`, `backups/`.
    state_dir: PathBuf,
    /// Сколько авто-бэкапов на игру хранить (plan §2.1, retention).
    backup_retention: u32,
}

impl<B: Backend> Engine<B> {
    pub fn new(backend: B, device_id: impl Into<String>, state_dir: impl Into<PathBuf>) -> Self {
        Engine {
            backend,
            history_channel: None,
            device_id: device_id.into(),
            device_name: None,
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

    pub fn with_history_channel(mut self, channel: &'static str) -> Self {
        self.history_channel = Some(channel);
        self
    }

    pub fn with_device_name(mut self, name: &str) -> Self {
        let name = name.trim();
        self.device_name = (!name.is_empty()).then(|| name.chars().take(128).collect());
        self
    }
}

mod import;
mod locks;
mod restore;
mod state;
#[cfg(test)]
#[path = "../tests/sync/engine.rs"]
mod tests;
mod upload;
pub use state::snapshot_details;

/// Recover an interrupted file restore before exposing profile state to clients.
pub fn recover_state(state: &Path) -> io::Result<()> {
    let _guard = file_transaction::state_guard(state)?;
    Ok(())
}
