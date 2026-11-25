//! SaveSync core: идентификация игр (slug'и), манифесты `savesync-manifests`,
//! сканер, снапшоты и локальный индекс версий.
//!
//! MVP (итерация 1): только ручная синхронизация, без автотриггеров.

pub mod game_key;
pub mod hash;
pub mod manifest;
pub mod manifest_store;
pub mod paths;
pub mod scanner;
pub mod snapshot;
pub mod steam;
pub mod storage_paths;

pub mod backend;
pub mod backup;
pub mod config;
pub mod credential_backend;
pub mod credentials;
pub mod engine;
pub mod gdrive;
pub mod index;
pub mod ipc;
pub mod lan;
pub mod oauth;
pub mod yandex;

pub mod bundled_manifests;

mod index_journal;

mod publication_cache;

pub mod updates;
