//! Бэкенды синхронизации: общий trait для облака (M2), LAN (M3) и
//! файловой реализации — она же служит эталонным backend'ом для
//! интеграционных тестов sync engine (выгрузка→загрузка «двух устройств»).
//!
//! Модель данных на бэкенде (plan §2):
//! ```text
//! <root>/index.json                  # реестр версий (CAS — TODO M2)
//! <root>/saves/<game_key>/<version_id>.zip
//! ```

use std::io;
use std::path::PathBuf;

/// Канал синхронизации. Реализации: FsBackend (M1, тесты/локальная папка),
/// YandexDisk/GoogleDrive (M2), LAN (M3). Все операции — целиком байтами,
/// атомарность `put_index` (CAS по etag/revision) — ответственность
/// реализации в M2; FsBackend пишет через tmp+rename.
pub trait Backend {
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()>;
    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>>;
    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>>;
    fn get_index(&self) -> io::Result<Option<Vec<u8>>>;
    fn put_index(&self, data: &[u8]) -> io::Result<()>;
    /// Publish version history without dropping concurrent publications.
    fn publish_index(&self, data: &[u8], _writer: &str) -> io::Result<()> {
        self.put_index(data)
    }
    fn name(&self) -> &'static str;
    fn verify_access(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "access check is unavailable",
        ))
    }
}

/// Каналы независимы (plan §3): движок может работать поверх набора
/// бэкендов, поэтому trait object тоже реализует Backend.
impl Backend for Box<dyn Backend> {
    fn publish_index(&self, data: &[u8], writer: &str) -> io::Result<()> {
        (**self).publish_index(data, writer)
    }
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()> {
        (**self).put_snapshot(game_key, version_id, data)
    }
    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>> {
        (**self).get_snapshot(game_key, version_id)
    }
    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>> {
        (**self).list_snapshots(game_key)
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        (**self).get_index()
    }
    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        (**self).put_index(data)
    }
    fn name(&self) -> &'static str {
        (**self).name()
    }
}

/// Файловый бэкенд: директория с моделью данных облака. Используется в
/// тестах и как «ручной канал» (USB/расшаренная папка) без аккаунтов.
pub struct FsBackend {
    root: PathBuf,
}

impl FsBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsBackend { root: root.into() }
    }

    fn snapshot_path(&self, game_key: &str, version_id: &str) -> io::Result<PathBuf> {
        crate::storage::storage_paths::validate_version(version_id)?;
        Ok(self
            .root
            .join("saves")
            .join(crate::storage::storage_paths::game_component(game_key)?)
            .join(format!("{version_id}.zip")))
    }
}

impl FsBackend {
    fn publish(&self, data: &[u8], writer: Option<&str>) -> io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("index.lock"))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let incoming = crate::sync::index_journal::parse(data)?;
        let old = self.get_index()?;
        let mut merged = old
            .as_deref()
            .map(crate::sync::index_journal::parse)
            .transpose()?
            .unwrap_or_else(crate::sync::index::Index::new);
        let previous_locks = merged.locks.clone();
        crate::sync::index_journal::merge(&mut merged, &incoming)?;
        for (game, entry) in &incoming.games {
            merged.game_mut(game).policy = entry.policy;
        }
        if let Some(writer) = writer {
            merged.locks = previous_locks;
            merged.locks.retain(|_, lock| lock.device != writer);
            for (game, lock) in &incoming.locks {
                if lock.device == writer {
                    merged.locks.insert(game.clone(), lock.clone());
                }
            }
        } else {
            merged.locks = incoming.locks;
        }
        let bytes = if old.is_none() {
            data.to_vec()
        } else {
            serde_json::to_vec(&merged).map_err(io::Error::other)?
        };
        let path = self.root.join("index.json");
        let temporary = self.root.join("index.json.tmp");
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, path)
    }
}

impl Backend for FsBackend {
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()> {
        let path = self.snapshot_path(game_key, version_id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let guard = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("zip.lock"))?;
        fs2::FileExt::lock_exclusive(&guard)?;
        match std::fs::read(&path) {
            Ok(existing) if existing == data => return Ok(()),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "immutable snapshot already exists with different content",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        let tmp = path.with_extension("zip.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &path)
    }
    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.snapshot_path(game_key, version_id)?)
    }

    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>> {
        let dir = self
            .root
            .join("saves")
            .join(crate::storage::storage_paths::game_component(game_key)?);
        let mut out = Vec::new();
        match std::fs::read_dir(&dir) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    let name = entry.file_name();
                    if let Some(s) = name.to_str().and_then(|s| s.strip_suffix(".zip")) {
                        out.push(s.to_string());
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        out.sort();
        Ok(out)
    }

    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        match std::fs::read(self.root.join("index.json")) {
            Ok(data) => Ok(Some(data)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        self.publish(data, None)
    }
    fn publish_index(&self, data: &[u8], writer: &str) -> io::Result<()> {
        self.publish(data, Some(writer))
    }

    fn name(&self) -> &'static str {
        "fs"
    }
}

#[cfg(test)]
#[path = "../tests/backends/backend.rs"]
mod tests;
