//! Клиентская сторона протокола (desktop app, CLI, Decky-прокси).

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::{prelude::*, GenericNamespaced, Name};

use super::{Method, Request, Response};

/// Имя сокета демона по умолчанию (shared с daemon crate).
pub const DEFAULT_SOCKET: &str = "savesync-daemon";

/// Override the IPC namespace for isolated tests or multiple profiles.
pub fn socket_name() -> String {
    std::env::var("SAVESYNC_SOCKET")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| DEFAULT_SOCKET.to_string())
}

fn ns_name(name: &str) -> std::io::Result<Name<'static>> {
    name.to_owned()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
}

/// Клиентский вызов: один запрос — одно соединение — один ответ.
pub fn call(name: &str, request: Request) -> std::io::Result<Response> {
    let mut stream = LocalSocketStream::connect(ns_name(name)?)?;
    super::authorize_peer(&stream)?;
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
