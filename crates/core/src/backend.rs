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
    fn name(&self) -> &'static str;
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

    fn snapshot_path(&self, game_key: &str, version_id: &str) -> PathBuf {
        self.root
            .join("saves")
            .join(game_key)
            .join(format!("{version_id}.zip"))
    }
}

impl Backend for FsBackend {
    fn put_snapshot(&self, game_key: &str, version_id: &str, data: &[u8]) -> io::Result<()> {
        let path = self.snapshot_path(game_key, version_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("zip.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &path)
    }

    fn get_snapshot(&self, game_key: &str, version_id: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.snapshot_path(game_key, version_id))
    }

    fn list_snapshots(&self, game_key: &str) -> io::Result<Vec<String>> {
        let dir = self.root.join("saves").join(game_key);
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
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join("index.json");
        let tmp = self.root.join("index.json.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &path)
    }

    fn name(&self) -> &'static str {
        "fs"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "savesync-test-backend-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn snapshot_roundtrip_and_listing() {
        let dir = fixture("roundtrip");
        let b = FsBackend::new(&dir);

        b.put_snapshot("elden-ring", "v1_d_aaaaaaaa", b"zip1")
            .unwrap();
        b.put_snapshot("elden-ring", "v2_d_bbbbbbbb", b"zip2")
            .unwrap();
        b.put_snapshot("hades", "v1_d_cccccccc", b"zip3").unwrap();

        assert_eq!(
            b.get_snapshot("elden-ring", "v1_d_aaaaaaaa").unwrap(),
            b"zip1"
        );
        assert_eq!(
            b.list_snapshots("elden-ring").unwrap(),
            vec!["v1_d_aaaaaaaa", "v2_d_bbbbbbbb"]
        );
        assert_eq!(b.list_snapshots("absent").unwrap(), Vec::<String>::new());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn index_roundtrip() {
        let dir = fixture("index");
        let b = FsBackend::new(&dir);
        assert!(b.get_index().unwrap().is_none());

        b.put_index(br#"{"version":1}"#).unwrap();
        assert_eq!(b.get_index().unwrap().unwrap(), br#"{"version":1}"#);

        std::fs::remove_dir_all(&dir).ok();
    }
}
