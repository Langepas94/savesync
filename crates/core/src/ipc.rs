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
