//! Контент-хеширование blake3: основа версионирования снапшотов.
//! Новая версия создаётся только при изменении хеша (см. plan §2.1).

use std::io;
use std::path::Path;

/// Хеш файла целиком (streaming, без загрузки в память).
pub fn hash_file(path: &Path) -> io::Result<blake3::Hash> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize())
}

/// Хеш буфера в памяти.
pub fn hash_bytes(data: &[u8]) -> blake3::Hash {
    blake3::hash(data)
}

/// Агрегированный хеш набора файлов снапшота: детерминирован по
/// отсортированному списку (относительный путь, хеш содержимого).
pub fn hash_snapshot(mut entries: Vec<(String, blake3::Hash)>) -> blake3::Hash {
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (path, hash) in entries {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
        hasher.update(hash.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_hash_is_stable() {
        assert_eq!(hash_bytes(b"save-data"), hash_bytes(b"save-data"));
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
    }

    #[test]
    fn file_hash_matches_bytes() {
        let path = std::env::temp_dir().join(format!("savesync-hash-{}", std::process::id()));
        std::fs::write(&path, b"content").unwrap();
        let h = hash_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(h, hash_bytes(b"content"));
    }

    #[test]
    fn snapshot_hash_is_order_independent() {
        let a = ("saves/a.sav".to_string(), hash_bytes(b"1"));
        let b = ("saves/b.sav".to_string(), hash_bytes(b"2"));
        assert_eq!(
            hash_snapshot(vec![a.clone(), b.clone()]),
            hash_snapshot(vec![b, a])
        );
    }

    #[test]
    fn snapshot_hash_changes_with_content() {
        let a = ("save.sav".to_string(), hash_bytes(b"v1"));
        let b = ("save.sav".to_string(), hash_bytes(b"v2"));
        assert_ne!(hash_snapshot(vec![a]), hash_snapshot(vec![b]));
    }
}
