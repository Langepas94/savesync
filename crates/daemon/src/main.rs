//! savesync-daemon — фоновый демон синхронизации сейвов.
//!
//! Команды:
//!   serve              запустить демон (IPC-сервер, unix socket / named pipe)
//!   status             статус (через демон, если запущен)
//!   scan               пересканировать игры
//!   sync [game_key]    ручная синхронизация (всех или одной игры)
//!
//! scan/sync/status сначала пробуют IPC к запущенному демону; если демон
//! не запущен — scan/sync выполняются напрямую (offline-режим CLI).

mod cli;
mod ipc;
mod service;

fn main() {
    std::process::exit(cli::run());
}
