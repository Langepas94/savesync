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
    /// Пересканировать систему, вернуть найденные игры.
    Scan,
    /// Ручная синхронизация (MVP-триггер). `None` — все игры.
    Sync { game_key: Option<String> },
    /// История версий игры (из локального индекса).
    History { game_key: String },
    /// Применить конкретную версию (откат / выбор ветки конфликта).
    Restore {
        game_key: String,
        version_id: String,
    },
    /// Переносимый бэкап игры в папку `backups.dir` (plan §2.2).
    Backup { game_key: String },
    /// Текущий конфиг (каналы, бэкапы, пути).
    GetConfig,
    /// Сохранить конфиг (каналы/бэкапы/пути). Демон перечитывает его
    /// на следующую операцию.
    SaveConfig { config: Box<crate::config::Config> },
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
    Status(StatusPayload),
    Scan(ScanPayload),
    Sync(SyncPayload),
    History(HistoryPayload),
    Backup(BackupPayload),
    Config(ConfigPayload),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPayload {
    pub game_key: String,
    pub versions: Vec<VersionEntry>,
    /// Применённая на этом устройстве версия.
    pub applied: Option<String>,
    /// «Головы» линии (больше одной = конфликт).
    pub heads: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionEntry {
    pub id: String,
    pub parent: Option<String>,
    pub created_at: u64,
    pub device: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupPayload {
    pub game_key: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigPayload {
    pub config: crate::config::Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPayload {
    pub daemon_version: String,
    pub device_id: String,
    pub device_name: String,
    pub games: Vec<GameStatus>,
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

/// Клиентская сторона протокола (desktop app, CLI, Decky-прокси).
pub mod client {
    use std::io::{BufRead, BufReader, Write};

    use interprocess::local_socket::{prelude::*, GenericNamespaced, Name};

    use super::{Method, Request, Response};

    /// Имя сокета демона по умолчанию (shared с daemon crate).
    pub const DEFAULT_SOCKET: &str = "savesync-daemon";

    fn ns_name(name: &str) -> std::io::Result<Name<'static>> {
        name.to_owned()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    }

    /// Клиентский вызов: один запрос — одно соединение — один ответ.
    pub fn call(name: &str, request: Request) -> std::io::Result<Response> {
        let mut stream = LocalSocketStream::connect(ns_name(name)?)?;
        stream.write_all(
            request
                .encode()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
                .as_bytes(),
        )?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "daemon closed connection",
            ));
        }
        Response::decode(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Есть ли живой демон на сокете.
    pub fn probe(name: &str) -> bool {
        call(name, Request::new(0, Method::Status)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req = Request::new(
            7,
            Method::Sync {
                game_key: Some("elden-ring".into()),
            },
        );
        let line = req.encode().unwrap();
        assert!(line.ends_with('\n'));
        let back = Request::decode(&line).unwrap();
        assert_eq!(back.id, 7);
        assert_eq!(back.method, req.method);
    }

    #[test]
    fn response_roundtrip_ok_and_err() {
        let ok = Response::ok(
            1,
            ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: "g".into(),
                    outcome: "uploaded".into(),
                    detail: None,
                }],
            }),
        );
        let back = Response::decode(&ok.encode().unwrap()).unwrap();
        assert!(back.result.is_some());
        assert!(back.error.is_none());

        let err = Response::err(2, "backend offline");
        let back = Response::decode(&err.encode().unwrap()).unwrap();
        assert_eq!(back.error.unwrap().message, "backend offline");
    }

    #[test]
    fn method_serde_shapes() {
        let status = serde_json::to_string(&Method::Status).unwrap();
        assert_eq!(status, r#"{"method":"status"}"#);
        let sync = serde_json::to_string(&Method::Sync { game_key: None }).unwrap();
        assert!(sync.contains(r#""method":"sync""#));
        assert!(sync.contains(r#""game_key":null"#));
    }
}
