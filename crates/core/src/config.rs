//! Конфигурация демона: пути данных per-platform, device_id, каналы
//! синхронизации. Файл — `<data_dir>/config.json`.
//!
//! Каналы независимы (plan §3): облако и LAN — отдельные блоки, можно
//! любой/оба/ни одного (тогда только локальные бэкапы). M1: только
//! `fs`-канал (локальная папка/USB) — облачные бэкенды в M2.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::Platform;

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
    /// Порт TCP-сервера для пиров.
    pub port: u16,
    /// Pairing PIN (в конфиге — открытым текстом; на проверку уходит
    /// только хеш). Защита от случайных подключений, не от MitM.
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
        if let Some(cfg) = Self::load(path)? {
            let legacy = cfg.backends.google.as_ref().is_some_and(|backend| {
                !backend.token.is_empty() && !crate::credentials::is_reference(&backend.token)
            }) || cfg.backends.yandex.as_ref().is_some_and(|backend| {
                !backend.token.is_empty() && !crate::credentials::is_reference(&backend.token)
            });
            if legacy {
                cfg.save(path)?;
                return Self::load(path)?.ok_or_else(|| {
                    io::Error::other("config disappeared during credential migration")
                });
            }
            return Ok(cfg);
        }
        let cfg = Config {
            version: 1,
            device_id: generate_device_id(),
            device_name: hostname(),
            manifests_dir: None,
            backends: BackendsConfig::default(),
            backups: BackupsSettings::default(),
        };
        cfg.save(path)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.save_with_store(path, &crate::credentials::OsSecretStore)
    }

    pub fn save_with_store(
        &self,
        path: &Path,
        store: &dyn crate::credentials::SecretStore,
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
                crate::oauth::Provider::Google,
                self.backends
                    .google
                    .as_ref()
                    .map(|value| value.token.as_str()),
                old.as_ref()
                    .and_then(|config| config.backends.google.as_ref())
                    .map(|value| value.token.as_str()),
            ),
            (
                crate::oauth::Provider::Yandex,
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
                crate::credentials::is_reference(reference) && Some(*reference) != old_ref
            }) {
                crate::credentials::load(store,reference,provider).map_err(|_|io::Error::new(io::ErrorKind::PermissionDenied,"credential reference is unavailable or stale; reload settings and sign in again"))?;
            }
        }
        let secured = crate::credentials::secure_config(self, store)?;
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
        let new_refs = crate::credentials::references(&secured);
        if result.is_err() {
            let original_refs = crate::credentials::references(self);
            for reference in new_refs {
                if !original_refs.contains(&reference) {
                    let _ = crate::credentials::delete(store, reference);
                }
            }
        } else if let Some(old) = old {
            for reference in crate::credentials::references(&old) {
                if !new_refs.contains(&reference) {
                    let _ = crate::credentials::delete(store, reference);
                }
            }
        }
        result
    }
}

/// Читаемое имя хоста (COMPUTERNAME / HOSTNAME), fallback — "device".
pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "device".into())
}

/// Стабильный id: slug(hostname) + 8 hex от blake3(время, pid).
/// Криптостойкость не требуется — это метка устройства, а не секрет.
pub fn generate_device_id() -> String {
    let host = crate::game_key::slugify(&hostname());
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
mod tests {
    use super::*;

    #[test]
    fn init_persists_device_id() {
        let dir = std::env::temp_dir().join(format!("savesync-test-config-{}", std::process::id()));
        let path = dir.join("config.json");

        let cfg = Config::load_or_init(&path).unwrap();
        assert_eq!(cfg.version, 1);
        assert!(!cfg.device_id.is_empty());

        // Повторная загрузка — тот же device_id (стабильность).
        let cfg2 = Config::load_or_init(&path).unwrap();
        assert_eq!(cfg, cfg2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn device_id_is_slug_prefixed() {
        let id = generate_device_id();
        let (host, suffix) = id.rsplit_once('-').unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(crate::game_key::is_valid_slug(host) || host == "device");
    }

    #[test]
    fn backends_are_optional_and_independent() {
        let json =
            r#"{"version":1,"device_id":"pc-abcd1234","device_name":"PC","manifests_dir":null}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(cfg.backends.fs_path.is_none());
    }
}
