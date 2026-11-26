//! Конфигурация демона: пути данных per-platform, device_id, каналы
//! синхронизации. Файл — `<data_dir>/config.json`.
//!
//! Каналы независимы (plan §3): облако и LAN — отдельные блоки, можно
//! любой/оба/ни одного (тогда только локальные бэкапы). M1: только
//! `fs`-канал (локальная папка/USB) — облачные бэкенды в M2.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform::paths::Platform;

/// Директория данных приложения на этой машине.
/// Windows: `%APPDATA%\savesync`, Linux/SteamOS: `~/.local/share/savesync`.
pub fn data_dir(platform: Platform) -> Option<PathBuf> {
    match platform {
        Platform::Windows => std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("savesync")),
        Platform::Linux => std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|p| p.join("savesync")),
    }
}

/// Настройки бэкендов (каналов). Всё опционально и независимо.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendsConfig {
    /// Файловый канал M1 (папка/USB). Также эталон для тестов.
    pub fs_path: Option<PathBuf>,
    /// Яндекс.Диск (M2): OAuth-токен. Каналы независимы — можно
    /// настроить любой набор (plan §3).
    pub yandex: Option<YandexConfig>,
    /// Google Drive (M2): OAuth access token. Refresh-flow — после
    /// получения client id/secret (plan §6 «OAuth»).
    pub google: Option<GoogleConfig>,
    /// LAN-канал (M3): это устройство — сервер для пиров + список пиров,
    /// с которыми синкаемся напрямую.
    pub lan: Option<LanConfig>,
}

/// LAN-канал (plan §3). PIN — общий секрет pairing'а; на сервере
/// хранится как blake3-хеш.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanConfig {
    /// QUIC/UDP listener port.
    pub port: u16,
    /// Legacy setting retained for config compatibility. Enrollment now uses a
    /// random, short-lived PIN and explicit SAS confirmation; this value grants no access.
    pub pin: String,
    /// Адреса пиров (`host:port`), с которыми синкаемся напрямую.
    #[serde(default)]
    pub peers: Vec<String>,
}

/// Канал Яндекс.Диска.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YandexConfig {
    /// OAuth-токен (scope cloud-api:disk.app_folder или disk).
    pub token: String,
}

/// Канал Google Drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleConfig {
    /// OAuth access token (scope drive.appdata).
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    /// Стабильный идентификатор устройства (генерируется при первом
    /// запуске). Участвует в id версий и pairing.
    pub device_id: String,
    /// Человекочитаемое имя устройства для UI/pairing.
    pub device_name: String,
    /// Снапшот репозитория savesync-manifests (директория с деревом).
    pub manifests_dir: Option<PathBuf>,
    #[serde(default)]
    pub backends: BackendsConfig,
    #[serde(default)]
    pub backups: BackupsSettings,
}

/// Настройки пользовательских бэкапов (plan §2.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupsSettings {
    /// Папка бэкапов по умолчанию (для `backup create` без аргумента и
    /// авто-бэкапов по расписанию).
    pub dir: Option<PathBuf>,
    /// Retention: сколько файлов бэкапов на игру хранить (по умолчанию 10).
    pub keep: Option<u32>,
    /// Интервал авто-бэкапов в часах (None — выключены; serve проверяет
    /// раз в полчаса и делает бэкап, если интервал прошёл).
    pub auto_interval_hours: Option<u32>,
}

impl BackupsSettings {
    fn for_profile(path: &Path) -> Self {
        Self {
            dir: Some(
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("backups"),
            ),
            keep: Some(10),
            auto_interval_hours: Some(24),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> io::Result<Option<Config>> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Загружает конфиг или создаёт новый (генерируя device_id).
    pub fn load_or_init(path: &Path) -> io::Result<Config> {
        if let Some(mut cfg) = Self::load(path)? {
            let legacy = cfg.backends.google.as_ref().is_some_and(|backend| {
                !backend.token.is_empty() && !crate::auth::credentials::is_reference(&backend.token)
            }) || cfg.backends.yandex.as_ref().is_some_and(|backend| {
                !backend.token.is_empty() && !crate::auth::credentials::is_reference(&backend.token)
            });
            let initialize_backups = cfg.backups.dir.is_none();
            if initialize_backups {
                let defaults = BackupsSettings::for_profile(path);
                if cfg.backups == BackupsSettings::default() {
                    cfg.backups = defaults;
                } else {
                    cfg.backups.dir = defaults.dir;
                }
            }
            if legacy || initialize_backups {
                cfg.save(path)?;
                if initialize_backups {
                    if let Some(directory) = &cfg.backups.dir {
                        let _ = std::fs::create_dir_all(directory);
                    }
                }
                return Self::load(path)?
                    .ok_or_else(|| io::Error::other("config disappeared during migration"));
            }
            return Ok(cfg);
        }
        let cfg = Config {
            version: 1,
            device_id: generate_device_id(),
            device_name: hostname(),
            manifests_dir: None,
            backends: BackendsConfig::default(),
            backups: BackupsSettings::for_profile(path),
        };
        cfg.save(path)?;
        if let Some(directory) = &cfg.backups.dir {
            let _ = std::fs::create_dir_all(directory);
        }
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.save_with_store(path, &crate::auth::credentials::OsSecretStore)
    }

    pub fn save_with_store(
        &self,
        path: &Path,
        store: &dyn crate::auth::credentials::SecretStore,
    ) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("json.lock"))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let old = Self::load(path)?;
        // A stale UI must not replace a live credential with a deleted reference.
        for (provider, new_ref, old_ref) in [
            (
                crate::auth::oauth::Provider::Google,
                self.backends
                    .google
                    .as_ref()
                    .map(|value| value.token.as_str()),
                old.as_ref()
                    .and_then(|config| config.backends.google.as_ref())
                    .map(|value| value.token.as_str()),
            ),
            (
                crate::auth::oauth::Provider::Yandex,
                self.backends
                    .yandex
                    .as_ref()
                    .map(|value| value.token.as_str()),
                old.as_ref()
                    .and_then(|config| config.backends.yandex.as_ref())
                    .map(|value| value.token.as_str()),
            ),
        ] {
            if let Some(reference) = new_ref.filter(|reference| {
                crate::auth::credentials::is_reference(reference) && Some(*reference) != old_ref
            }) {
                crate::auth::credentials::load(store,reference,provider).map_err(|_|io::Error::new(io::ErrorKind::PermissionDenied,"credential reference is unavailable or stale; reload settings and sign in again"))?;
            }
        }
        let secured = crate::auth::credentials::secure_config(self, store)?;
        let result = (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(
                &tmp,
                serde_json::to_vec_pretty(&secured).map_err(io::Error::other)?,
            )?;
            std::fs::rename(&tmp, path)
        })();
        let new_refs = crate::auth::credentials::references(&secured);
        if result.is_err() {
            let original_refs = crate::auth::credentials::references(self);
            for reference in new_refs {
                if !original_refs.contains(&reference) {
                    let _ = crate::auth::credentials::delete(store, reference);
                }
            }
        } else if let Some(old) = old {
            for reference in crate::auth::credentials::references(&old) {
                if !new_refs.contains(&reference) {
                    let _ = crate::auth::credentials::delete(store, reference);
                }
            }
        }
        result
    }
}

/// Читаемое имя хоста (COMPUTERNAME / HOSTNAME), fallback — "device".
pub fn hostname() -> String {
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Ok(value) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        if !value.trim().is_empty() {
            return value.trim().to_owned();
        }
    }
    "device".into()
}

/// Стабильный id: slug(hostname) + 8 hex от blake3(время, pid).
/// Криптостойкость не требуется — это метка устройства, а не секрет.
pub fn generate_device_id() -> String {
    let host = crate::catalog::game_key::slugify(&hostname());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(host.as_bytes());
    let short: String = hasher.finalize().to_hex().chars().take(8).collect();
    let host = if host.is_empty() {
        "device".into()
    } else {
        host
    };
    format!("{host}-{short}")
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
