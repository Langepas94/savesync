//! Локальный IPC-сервер и адаптация команд к сценариям демона.

pub(crate) mod dispatch;
mod server;

pub(crate) use server::{call, probe, serve};
