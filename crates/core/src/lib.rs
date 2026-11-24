//! SaveSync core: идентификация игр (slug'и), манифесты `savesync-manifests`,
//! сканер, снапшоты и локальный индекс версий.
//!
//! MVP (итерация 1): только ручная синхронизация, без автотриггеров.

pub mod game_key;
pub mod hash;
pub mod manifest;
pub mod paths;
pub mod scanner;
pub mod snapshot;
pub mod steam;

pub mod backend;
pub mod config;
pub mod engine;
pub mod index;
pub mod ipc;
pub mod yandex;
