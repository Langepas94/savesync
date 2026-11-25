//! LAN-канал (plan §3): устройство выступает бэкендом для пиров.
//!
//! Модель: на устройстве-«сервере» есть локальное хранилище
//! `<state>/lan-store` (та же модель данных, что у FsBackend); его же
//! движок синкается с ним через FsBackend — а пиры подключаются по TCP
//! через [`LanBackend`]. Так «облако» и «LAN» остаются независимыми
//! каналами одного sync engine.
//!
//! Протокол: TCP, кадры `[u32 LE len][payload]`. Запрос — JSON-заголовок
//! (`LanRequest`), при наличии бинарных данных (`put_index`,
//! `put_snapshot`) следом идёт второй кадр с сырыми байтами. Ответ —
//! JSON (`LanResponse`), при `blob_len > 0` за ним кадр с байтами.
//!
//! Аутентификация — pairing PIN (plan §3): клиент присылает blake3(pin),
//! сервер сверяет со своим хешем. Трафик не шифруется (LAN, итерация 2 —
//! Noise/TLS); PIN защищает от случайных подключений, не от активного
//! MitM в локальной сети.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::{Backend, FsBackend};
mod server;
pub use server::LanServer;

/// Кадр больше 512 МБ не принимаем (снапшоты меньше).
const MAX_FRAME: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum LanRequest {
    /// Проверка связи и PIN.
    Ping {
        pin_hash: String,
    },
    GetIndex {
        pin_hash: String,
    },
    /// За заголовком следует кадр с `blob_len` байтами.
    PutIndex {
        pin_hash: String,
        blob_len: u64,
    },
    ListSnapshots {
        pin_hash: String,
        game_key: String,
    },
    GetSnapshot {
        pin_hash: String,
        game_key: String,
        version_id: String,
    },
    /// За заголовком следует кадр с `blob_len` байтами.
    PutSnapshot {
        pin_hash: String,
        game_key: String,
        version_id: String,
        blob_len: u64,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct LanResponse {
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list: Option<Vec<String>>,
    /// Сколько байт следует за этим ответом (get_index/get_snapshot).
    #[serde(default)]
    blob_len: u64,
}

impl LanResponse {
    fn ok() -> Self {
        LanResponse {
            ok: true,
            error: None,
            list: None,
            blob_len: 0,
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        LanResponse {
            ok: false,
            error: Some(msg.into()),
            list: None,
            blob_len: 0,
        }
    }
}

/// blake3-хеш PIN для сравнения на сервере (PIN не храним и не шлем
/// открытым текстом).
pub fn pin_hash(pin: &str) -> String {
    blake3::hash(pin.as_bytes()).to_hex().to_string()
}

fn read_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as u64;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn write_frame(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)?;
    w.flush()
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

// ---------------------------------------------------------------------------
// Сервер
// ---------------------------------------------------------------------------

/// TCP-сервер, обслуживающий `store_dir` (модель данных FsBackend) для
/// пиров. Блокирующий: запускать в отдельном потоке. Одно соединение =
/// один запрос.
pub fn serve(store_dir: PathBuf, port: u16, expected_pin_hash: String) -> io::Result<()> {
    // Compatibility entry point; managed daemon listeners use LanServer.
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    for stream in listener.incoming().flatten() {
        let _ = handle_conn(stream, &store_dir, &expected_pin_hash);
    }
    Ok(())
}

fn handle_conn(mut stream: TcpStream, store_dir: &PathBuf, pin_hash: &str) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let backend = FsBackend::new(store_dir);

    let mut head_len = [0u8; 4];
    stream.read_exact(&mut head_len)?;
    let head_len = u32::from_le_bytes(head_len) as usize;
    if head_len > 64 * 1024 {
        return Err(invalid("LAN header too large"));
    }
    let mut head = vec![0; head_len];
    stream.read_exact(&mut head)?;
    let req: LanRequest = serde_json::from_slice(&head).map_err(|e| invalid(e.to_string()))?;

    let supplied_pin = match &req {
        LanRequest::Ping { pin_hash }
        | LanRequest::GetIndex { pin_hash }
        | LanRequest::PutIndex { pin_hash, .. }
        | LanRequest::ListSnapshots { pin_hash, .. }
        | LanRequest::GetSnapshot { pin_hash, .. }
        | LanRequest::PutSnapshot { pin_hash, .. } => pin_hash,
    };
    if supplied_pin != pin_hash {
        return write_frame(
            &mut stream,
            &serde_json::to_vec(&LanResponse::err("bad pin"))?,
        );
    }

    // Для put-* сначала дочитываем бинарный кадр (чтобы стороны не
    // разошлись по фреймингу), затем проверяем PIN и только потом
    // применяем запись — неверный PIN не имеет побочных эффектов.
    let (req_pin, blob) = match &req {
        LanRequest::Ping { pin_hash } => (pin_hash.as_str(), None),
        LanRequest::GetIndex { pin_hash } => (pin_hash.as_str(), None),
        LanRequest::PutIndex { pin_hash, blob_len } => {
            let data = read_blob(&mut stream, *blob_len)?;
            (pin_hash.as_str(), Some(data))
        }
        LanRequest::ListSnapshots { pin_hash, .. } => (pin_hash.as_str(), None),
        LanRequest::GetSnapshot { pin_hash, .. } => (pin_hash.as_str(), None),
        LanRequest::PutSnapshot {
            pin_hash, blob_len, ..
        } => {
            let data = read_blob(&mut stream, *blob_len)?;
            (pin_hash.as_str(), Some(data))
        }
    };

    if req_pin != pin_hash {
        let resp =
            serde_json::to_vec(&LanResponse::err("bad pin")).map_err(|e| invalid(e.to_string()))?;
        return write_frame(&mut stream, &resp);
    }

    let (response, out_blob): (LanResponse, Option<Vec<u8>>) = match &req {
        LanRequest::Ping { .. } => (LanResponse::ok(), None),
        LanRequest::GetIndex { .. } => match backend.get_index() {
            Ok(Some(bytes)) => {
                let mut r = LanResponse::ok();
                r.blob_len = bytes.len() as u64;
                (r, Some(bytes))
            }
            Ok(None) => (LanResponse::ok(), None),
            Err(e) => (LanResponse::err(e.to_string()), None),
        },
        LanRequest::PutIndex { .. } => {
            let data = blob.expect("blob read above");
            (
                match backend.put_index(&data) {
                    Ok(()) => LanResponse::ok(),
                    Err(e) => LanResponse::err(e.to_string()),
                },
                None,
            )
        }
        LanRequest::ListSnapshots { game_key, .. } => (
            match backend.list_snapshots(game_key) {
                Ok(list) => LanResponse {
                    list: Some(list),
                    ..LanResponse::ok()
                },
                Err(e) => LanResponse::err(e.to_string()),
            },
            None,
        ),
        LanRequest::GetSnapshot {
            game_key,
            version_id,
            ..
        } => match backend.get_snapshot(game_key, version_id) {
            Ok(bytes) => {
                let mut r = LanResponse::ok();
                r.blob_len = bytes.len() as u64;
                (r, Some(bytes))
            }
            Err(e) => (LanResponse::err(e.to_string()), None),
        },
        LanRequest::PutSnapshot {
            game_key,
            version_id,
            ..
        } => {
            let data = blob.expect("blob read above");
            (
                match backend.put_snapshot(game_key, version_id, &data) {
                    Ok(()) => LanResponse::ok(),
                    Err(e) => LanResponse::err(e.to_string()),
                },
                None,
            )
        }
    };

    let bytes = serde_json::to_vec(&response).map_err(|e| invalid(e.to_string()))?;
    write_frame(&mut stream, &bytes)?;
    if let Some(blob) = out_blob {
        write_frame(&mut stream, &blob)?;
    }
    Ok(())
}

fn read_blob(stream: &mut TcpStream, expected: u64) -> io::Result<Vec<u8>> {
    let data = read_frame(stream)?;
    if data.len() as u64 != expected {
        return Err(invalid(format!(
            "blob size mismatch: header {expected}, frame {}",
            data.len()
        )));
    }
    Ok(data)
}

// ---------------------------------------------------------------------------
// Клиент (Backend)
// ---------------------------------------------------------------------------

/// LAN-бэкенд: синхронизация напрямую с пиром по TCP.
pub struct LanBackend {
    addr: String,
    pin_hash: String,
}

impl LanBackend {
    pub fn new(addr: impl Into<String>, pin: &str) -> Self {
        LanBackend {
            addr: addr.into(),
            pin_hash: pin_hash(pin),
        }
    }

    /// Проверка связи и PIN.
    pub fn ping(&self) -> io::Result<()> {
        self.roundtrip(&LanRequest::Ping {
            pin_hash: self.pin_hash.clone(),
        })
        .map(|_| ())
    }

    /// Запрос → (ответ, опциональный blob).
    fn roundtrip(&self, req: &LanRequest) -> io::Result<(LanResponse, Option<Vec<u8>>)> {
        let mut stream = TcpStream::connect(&self.addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        stream.set_nodelay(true).ok();

        let head = serde_json::to_vec(req).map_err(|e| invalid(e.to_string()))?;
        write_frame(&mut stream, &head)?;

        let resp_bytes = read_frame(&mut stream)?;
        let resp: LanResponse =
            serde_json::from_slice(&resp_bytes).map_err(|e| invalid(e.to_string()))?;
        let blob = if resp.blob_len > 0 {
            Some(read_blob(&mut stream, resp.blob_len)?)
        } else {
            None
        };
        if !resp.ok {
            let msg = resp.error.unwrap_or_else(|| "unknown error".into());
            let kind = if msg.contains("not found") || msg.contains("NotFound") {
                io::ErrorKind::NotFound
            } else if msg == "bad pin" {
                io::ErrorKind::PermissionDenied
            } else {
                io::ErrorKind::Other
            };
            return Err(io::Error::new(kind, format!("lan {}: {msg}", self.addr)));
        }
        Ok((resp, blob))
    }

    /// put-*: заголовок, затем бинарный кадр.
    fn roundtrip_put(&self, req: &LanRequest, blob: &[u8]) -> io::Result<()> {
        let mut stream = TcpStream::connect(&self.addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        stream.set_nodelay(true).ok();
        let head = serde_json::to_vec(req).map_err(|e| invalid(e.to_string()))?;
        write_frame(&mut stream, &head)?;
        write_frame(&mut stream, blob)?;

        let resp_bytes = read_frame(&mut stream)?;
        let resp: LanResponse =
            serde_json::from_slice(&resp_bytes).map_err(|e| invalid(e.to_string()))?;
        if !resp.ok {
            return Err(io::Error::other(format!(
                "lan {}: {}",
                self.addr,
                resp.error.unwrap_or_default()
            )));
        }
        Ok(())
    }
}

impl Backend for LanBackend {
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()> {
        self.roundtrip_put(
            &LanRequest::PutSnapshot {
                pin_hash: self.pin_hash.clone(),
                game_key: game_key.to_string(),
                version_id: version_id.to_string(),
                blob_len: data.len() as u64,
            },
            data,
        )
    }

    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>> {
        let (resp, blob) = self.roundtrip(&LanRequest::GetSnapshot {
            pin_hash: self.pin_hash.clone(),
            game_key: game_key.to_string(),
            version_id: version_id.to_string(),
        })?;
        blob.ok_or_else(|| {
            if resp.blob_len == 0 {
                io::Error::new(io::ErrorKind::NotFound, format!("{game_key}/{version_id}"))
            } else {
                invalid("missing blob")
            }
        })
    }

    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>> {
        let (resp, _) = self.roundtrip(&LanRequest::ListSnapshots {
            pin_hash: self.pin_hash.clone(),
            game_key: game_key.to_string(),
        })?;
        Ok(resp.list.unwrap_or_default())
    }

    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        let (_resp, blob) = self.roundtrip(&LanRequest::GetIndex {
            pin_hash: self.pin_hash.clone(),
        })?;
        Ok(blob)
    }

    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        self.roundtrip_put(
            &LanRequest::PutIndex {
                pin_hash: self.pin_hash.clone(),
                blob_len: data.len() as u64,
            },
            data,
        )
    }

    fn name(&self) -> &'static str {
        "lan"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Server {
        dir: PathBuf,
        port: u16,
    }

    impl Server {
        fn start(name: &str, pin: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "savesync-test-lan-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();

            // Свободный порт: биндимся на 0 и отпускаем.
            let port = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let store = dir.clone();
            let hash = pin_hash(pin);
            std::thread::spawn(move || {
                let _ = serve(store, port, hash);
            });
            // Дадим серверу подняться.
            std::thread::sleep(Duration::from_millis(100));
            Server { dir, port }
        }

        fn backend(&self, pin: &str) -> LanBackend {
            LanBackend::new(format!("127.0.0.1:{}", self.port), pin)
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    #[test]
    fn lan_roundtrip_and_pin_auth() {
        let srv = Server::start("roundtrip", "1234");
        let b = srv.backend("1234");

        b.ping().unwrap();
        assert!(b.get_index().unwrap().is_none());
        b.put_index(br#"{"version":1}"#).unwrap();
        assert_eq!(b.get_index().unwrap().unwrap(), br#"{"version":1}"#);

        b.put_snapshot("elden-ring", "v1_d_aaaaaaaa", b"zip1")
            .unwrap();
        b.put_snapshot("elden-ring", "v2_d_bbbbbbbb", b"zip2")
            .unwrap();
        assert_eq!(
            b.list_snapshots("elden-ring").unwrap(),
            vec!["v1_d_aaaaaaaa", "v2_d_bbbbbbbb"]
        );
        assert_eq!(
            b.get_snapshot("elden-ring", "v1_d_aaaaaaaa").unwrap(),
            b"zip1"
        );
        assert!(b.list_snapshots("absent").unwrap().is_empty());

        // Неверный PIN — PermissionDenied, и сервер остаётся жив.
        let bad = srv.backend("0000");
        let err = bad.ping().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(srv.backend("1234").ping().is_ok());
    }

    #[test]
    fn engine_over_lan_two_devices() {
        let srv = Server::start("engine", "pin");

        // «Устройство» pc пушит сейв в lan-store сервера через сеть.
        let dir =
            std::env::temp_dir().join(format!("savesync-test-lan-eng-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save_dir = dir.join("home/game/saves");
        std::fs::create_dir_all(&save_dir).unwrap();
        std::fs::write(save_dir.join("slot1.sav"), "progress").unwrap();

        let loc = crate::scanner::SaveLocation {
            location: None,
            pattern: "<home>/game/saves".into(),
            resolved: save_dir.to_string_lossy().replace('\\', "/"),
            hits: vec![save_dir.clone()],
        };
        let pc = crate::engine::Engine::new(srv.backend("pin"), "pc", dir.join("state-pc"));
        assert!(matches!(
            pc.push("test-game", &[loc]).unwrap(),
            crate::engine::PushOutcome::Uploaded(_)
        ));

        // deck подтягивает с того же сервера.
        let deck = crate::engine::Engine::new(srv.backend("pin"), "deck", dir.join("state-deck"));
        assert!(deck.has_remote_versions("test-game").unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }
}
