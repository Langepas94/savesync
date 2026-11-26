//! IPC-протокол демона: JSON-RPC поверх unix socket (Linux) / named pipe
//! (Windows), newline-delimited JSON. Клиенты: desktop app (M4), Decky
//! (M5, через Python-прокси), CLI самого демона.
//!
//! Запрос:  `{"id": 1, "method": "sync", "params": {...}}\n`
//! Ответ:   `{"id": 1, "result": {...}}` или `{"id": 1, "error": {...}}`

use serde::{Deserialize, Serialize};

/// Методы протокола.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Method {
    /// Состояние демона и игр (для трея/шторки).
    Status,
    /// Stop accepting work after active runtime operations finish.
    Shutdown,
    /// Пересканировать систему, вернуть найденные игры.
    Scan,
    /// Reload installed/curated/custom manifests and rescan without restart.
    ReloadManifests,
    AddCustomGame {
        name: String,
        paths: Vec<String>,
    },
    SetGameHidden {
        game_key: String,
        hidden: bool,
    },
    Lan {
        action: crate::backends::lan::LanAction,
    },
    LanPresence,
    OAuthBegin {
        provider: crate::auth::oauth::Provider,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
    },
    OAuthPoll {
        provider: crate::auth::oauth::Provider,
    },
    OAuthCancel {
        provider: crate::auth::oauth::Provider,
    },
    CloudCheck {
        provider: crate::auth::oauth::Provider,
    },
    /// Ручная синхронизация (MVP-триггер). `None` — все игры.
    Sync {
        game_key: Option<String>,
    },
    /// История версий игры (из локального индекса).
    History {
        game_key: String,
    },
    /// Применить конкретную версию (откат / выбор ветки конфликта).
    Restore {
        game_key: String,
        version_id: String,
    },
    /// Переносимый бэкап игры в папку `backups.dir` (plan §2.2).
    Backup {
        game_key: String,
    },
    BackupAll,
    /// Текущий конфиг (каналы, бэкапы, пути).
    GetConfig,
    /// Сохранить конфиг (каналы/бэкапы/пути). Демон перечитывает его
    /// на следующую операцию.
    SaveConfig {
        config: Box<crate::config::Config>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub method: Method,
}

impl Request {
    pub fn new(id: u64, method: Method) -> Self {
        Request { id, method }
    }

    /// Кодирует запрос строкой протокола (JSON + \n).
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        Ok(serde_json::to_string(self)? + "\n")
    }

    pub fn decode(line: &str) -> Result<Request, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ResponsePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: ResponsePayload) -> Self {
        Response {
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, message: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(RpcError {
                message: message.into(),
            }),
        }
    }

    pub fn encode(&self) -> Result<String, serde_json::Error> {
        Ok(serde_json::to_string(self)? + "\n")
    }

    pub fn decode(line: &str) -> Result<Response, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ResponsePayload {
    Stopped,
    Lan(crate::backends::lan::LanPayload),
    LanPresence(Vec<String>),
    OAuthStart(OAuthStartPayload),
    OAuthStatus(OAuthStatusPayload),
    Status(StatusPayload),
    Scan(ScanPayload),
    Sync(SyncPayload),
    History(HistoryPayload),
    Backup(BackupPayload),
    BackupBatch(BackupBatchPayload),
    Config(ConfigPayload),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthStartPayload {
    pub provider: crate::auth::oauth::Provider,
    pub authorization_url: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthStatusPayload {
    pub provider: crate::auth::oauth::Provider,
    pub state: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPayload {
    pub game_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install_paths: Vec<String>,
    #[serde(default)]
    pub local_files: Vec<LocalSaveFile>,
    pub versions: Vec<VersionEntry>,
    /// Применённая на этом устройстве версия.
    pub applied: Option<String>,
    /// «Головы» линии (больше одной = конфликт).
    pub heads: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSaveFile {
    pub path: String,
    pub size: u64,
    pub modified_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionEntry {
    pub id: String,
    pub parent: Option<String>,
    pub created_at: u64,
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupPayload {
    pub game_key: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupBatchPayload {
    pub created: usize,
    pub directory: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSummary {
    pub created_at: u64,
    pub directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigPayload {
    pub config: crate::config::Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPayload {
    #[serde(default)]
    pub busy: bool,
    pub daemon_version: String,
    pub device_id: String,
    pub device_name: String,
    pub games: Vec<GameStatus>,
    #[serde(default)]
    pub hidden_games: Vec<GameStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backup: Option<BackupSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameStatus {
    pub game_key: String,
    pub display_name: String,
    pub versions: usize,
    /// Состояние относительно общего индекса.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanPayload {
    pub found: usize,
    pub games: Vec<ScannedGame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedGame {
    pub game_key: String,
    pub display_name: String,
    pub locations: usize,
    pub portability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPayload {
    pub results: Vec<SyncResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResult {
    pub game_key: String,
    /// uploaded | applied | up_to_date | unchanged | conflict | skipped | error
    pub outcome: String,
    pub detail: Option<String>,
}
