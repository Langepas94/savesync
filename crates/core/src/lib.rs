//! Каталог игр, синхронизация сейвов и системные интеграции SaveSync.

pub mod auth;
pub mod backends;
pub mod catalog;
pub mod config;
pub mod ipc;
pub mod platform;
pub mod storage;
pub mod sync;
pub mod updates;

// Совместимость со старыми путями публичного API.
pub use auth::{credential_backend, credentials, oauth};
pub use backends::{backend, gdrive, lan, yandex};
pub use catalog::{bundled_manifests, game_key, manifest, manifest_store, scanner};
pub use platform::{installation, paths, steam};
pub use storage::{backup, hash, snapshot, storage_paths};
pub use sync::{engine, index};
