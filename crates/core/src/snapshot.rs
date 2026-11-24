//! Снапшоты сейвов: иммутабельный zip-архив + `manifest.json`.
//!
//! Модель (plan §2.1): синхронизируются снапшоты, а не «живые» папки.
//! Новая версия = timestamp + изменённый blake3; пустые изменения версию
//! не создают (сравнение content_hash).
//!
//! Структура архива:
//! ```text
//! manifest.json          # SnapshotManifest
//! loc0/<rel>...          # файлы первой save-локации (пути относительные)
//! loc1/<rel>...
//! ```
//! zstd-сжатие — TODO M6 (зависимость требует C-тулчейн; пока deflate).

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hash;
use crate::scanner::SaveLocation;

pub const MANIFEST_NAME: &str = "manifest.json";

/// Метаданные снапшота (manifest.json внутри архива).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub game_key: String,
    pub device_id: String,
    /// Unix-время создания, миллисекунды.
    pub created_at: u64,
    /// Агрегированный blake3 всех файлов (см. hash::hash_snapshot).
    pub content_hash: String,
    pub locations: Vec<LocationManifest>,
}

/// Одна save-локация внутри снапшота.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationManifest {
    /// Паттерн из манифеста игры — по нему целевое устройство
    /// перерезолвит базу при восстановлении.
    pub pattern: String,
    /// Директория в архиве (`loc0`, `loc1`, ...).
    pub archive_dir: String,
    pub files: Vec<FileManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileManifest {
    /// Путь относительно базы локации, `/`-разделители.
    pub rel_path: String,
    pub size: u64,
    pub hash: String,
}

/// База локации: резолвнутый паттерн, обрезанный до первого glob-символа
/// по границе сегмента. Все файлы локации складываются относительно неё.
pub fn location_base(resolved: &str) -> PathBuf {
    let cut = resolved.find(['*', '?', '[']).unwrap_or(resolved.len());
    let prefix = &resolved[..cut];
    let dir_end = prefix.rfind('/').map(|i| i + 1).unwrap_or(0);
    PathBuf::from(&resolved[..dir_end])
}

/// Рекурсивно собирает файлы локации (hits могут быть файлами и папками).
fn collect_files(base: &Path, hit: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> io::Result<()> {
    if hit.is_dir() {
        for entry in std::fs::read_dir(hit)? {
            collect_files(base, &entry?.path(), out)?;
        }
    } else if hit.is_file() {
        let rel = hit
            .strip_prefix(base)
            .map(PathBuf::from)
            .unwrap_or_else(|_| hit.to_path_buf());
        out.push((rel, hit.to_path_buf()));
    }
    Ok(())
}

/// Создаёт снапшот из найденных локаций игры в файл `out`.
pub fn create(
    game_key: &str,
    device_id: &str,
    locations: &[SaveLocation],
    out: &Path,
) -> io::Result<SnapshotManifest> {
    let file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut manifest = SnapshotManifest {
        game_key: game_key.to_string(),
        device_id: device_id.to_string(),
        created_at: now_millis(),
        content_hash: String::new(),
        locations: Vec::new(),
    };
    let mut hash_entries = Vec::new();

    for (i, loc) in locations.iter().enumerate() {
        let archive_dir = format!("loc{i}");
        let base = location_base(&loc.resolved);
        let mut pairs = Vec::new();
        for hit in &loc.hits {
            collect_files(&base, hit, &mut pairs)?;
        }
        pairs.sort();

        let mut files = Vec::new();
        for (rel, abs) in pairs {
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let data = std::fs::read(&abs)?;
            let h = hash::hash_bytes(&data);
            hash_entries.push((format!("{archive_dir}/{rel_str}"), h));
            files.push(FileManifest {
                rel_path: rel_str.clone(),
                size: data.len() as u64,
                hash: h.to_hex().to_string(),
            });
            zip.start_file(format!("{archive_dir}/{rel_str}"), options)
                .map_err(to_io)?;
            zip.write_all(&data)?;
        }
        manifest.locations.push(LocationManifest {
            pattern: loc.pattern.clone(),
            archive_dir,
            files,
        });
    }

    manifest.content_hash = hash::hash_snapshot(hash_entries).to_hex().to_string();

    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    zip.start_file(MANIFEST_NAME, options).map_err(to_io)?;
    zip.write_all(&manifest_json)?;
    zip.finish().map_err(to_io)?;

    Ok(manifest)
}

/// Читает манифест снапшота из архива.
pub fn read_manifest(archive: &Path) -> io::Result<SnapshotManifest> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(to_io)?;
    let mut entry = zip.by_name(MANIFEST_NAME).map_err(to_io)?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf)?;
    serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Проверяет целостность: хеш каждого файла в архиве = хешу в манифесте.
pub fn verify(archive: &Path) -> io::Result<bool> {
    let manifest = read_manifest(archive)?;
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(to_io)?;

    for loc in &manifest.locations {
        for f in &loc.files {
            let name = format!("{}/{}", loc.archive_dir, f.rel_path);
            let mut entry = zip.by_name(&name).map_err(to_io)?;
            let mut hasher = blake3::Hasher::new();
            io::copy(&mut entry, &mut hasher)?;
            if hasher.finalize().to_hex().as_str() != f.hash {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Распаковывает одну локацию снапшота в `dest` (создаётся при
/// необходимости). Существующие файлы затираются — вызывающая сторона
/// обязана сделать авто-бэкап (см. engine).
pub fn extract_location(
    archive: &Path,
    location: &LocationManifest,
    dest: &Path,
) -> io::Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(to_io)?;

    for f in &location.files {
        let name = format!("{}/{}", location.archive_dir, f.rel_path);
        let mut entry = zip.by_name(&name).map_err(to_io)?;
        // Защита от zip-slip: относительный путь не должен выходить из dest.
        let rel = Path::new(&f.rel_path);
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsafe path in snapshot: {}", f.rel_path),
            ));
        }
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn to_io(e: zip::result::ZipError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "savesync-test-snap-{}-{}",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Fixture(dir)
        }
        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn make_game_dir(fx: &Fixture) -> SaveLocation {
        std::fs::create_dir_all(fx.path("game/saves/nested")).unwrap();
        std::fs::write(fx.path("game/saves/slot1.sav"), b"save-one").unwrap();
        std::fs::write(fx.path("game/saves/nested/slot2.sav"), b"save-two").unwrap();
        let resolved = fx.path("game/saves").to_string_lossy().replace('\\', "/");
        SaveLocation {
            pattern: "<home>/game/saves".into(),
            resolved: format!("{resolved}/**"),
            hits: vec![fx.path("game/saves")],
        }
    }

    #[test]
    fn base_strips_glob() {
        assert_eq!(
            location_base("C:/Users/g/AppData/Roaming/Game/saves/*.sav"),
            PathBuf::from("C:/Users/g/AppData/Roaming/Game/saves/")
        );
        assert_eq!(
            location_base("/home/deck/.config/game"),
            PathBuf::from("/home/deck/.config/")
        );
    }

    #[test]
    fn create_read_verify_roundtrip() {
        let fx = Fixture::new("roundtrip");
        let loc = make_game_dir(&fx);
        let archive = fx.path("snap.zip");

        let manifest = create("test-game", "dev1", &[loc], &archive).unwrap();
        assert_eq!(manifest.game_key, "test-game");
        assert_eq!(manifest.locations.len(), 1);
        assert_eq!(manifest.locations[0].files.len(), 2);
        assert!(!manifest.content_hash.is_empty());

        let loaded = read_manifest(&archive).unwrap();
        assert_eq!(loaded, manifest);
        assert!(verify(&archive).unwrap());
    }

    #[test]
    fn content_hash_changes_with_content() {
        let fx = Fixture::new("hash");
        let loc = make_game_dir(&fx);
        let m1 = create("g", "d", std::slice::from_ref(&loc), &fx.path("1.zip")).unwrap();
        std::fs::write(fx.path("game/saves/slot1.sav"), b"changed").unwrap();
        let m2 = create("g", "d", std::slice::from_ref(&loc), &fx.path("2.zip")).unwrap();
        assert_ne!(m1.content_hash, m2.content_hash);

        // Пересоздание без изменений — тот же хеш (новая версия не нужна).
        let m3 = create("g", "d", std::slice::from_ref(&loc), &fx.path("3.zip")).unwrap();
        assert_eq!(m2.content_hash, m3.content_hash);
    }

    #[test]
    fn extract_restores_files() {
        let fx = Fixture::new("extract");
        let loc = make_game_dir(&fx);
        let archive = fx.path("snap.zip");
        let manifest = create("g", "d", &[loc], &archive).unwrap();

        let dest = fx.path("restored");
        extract_location(&archive, &manifest.locations[0], &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("slot1.sav")).unwrap(), b"save-one");
        assert_eq!(
            std::fs::read(dest.join("nested/slot2.sav")).unwrap(),
            b"save-two"
        );
    }
}
