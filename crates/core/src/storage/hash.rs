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
#[path = "../tests/storage/hash.rs"]
mod tests;
