//! Пользовательские бэкапы (plan §2.2): переносимые файлы
//! `*.savesync-backup` (тот же zip-снапшот с manifest.json — файл
//! самодостаточен, импортируется на любом устройстве без pairing),
//! ротация retention и импорт как новой версии в линии игры.
//!
//! Локальные авто-бэкапы движка (перед затиранием при pull) — отдельный
//! механизм в engine (`<state>/backups/<game>/<ts>/`), ротируются через
//! [`prune_dir_backups`].

use std::io;
use std::path::{Path, PathBuf};

use crate::scanner::SaveLocation;
use crate::snapshot;

/// Расширение переносимого файла бэкапа.
pub const BACKUP_EXT: &str = "savesync-backup";

pub(crate) fn is_backup_archive(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(BACKUP_EXT))
}

/// Информация о файле бэкапа (из manifest.json внутри).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub game_key: String,
    pub device_id: String,
    pub created_at: u64,
    pub content_hash: String,
}

/// Создать переносимый бэкап игры в `dest_dir`. Возвращает путь файла.
/// Имя: `<slug>_<millis>.savesync-backup` (slug очищен от `custom:`).
pub fn create_backup(
    game_key: &str,
    device_id: &str,
    locations: &[SaveLocation],
    dest_dir: &Path,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dest_dir)?;
    let slug = crate::storage_paths::game_component(game_key)?;
    let file = dest_dir.join(format!("{slug}_{}.{BACKUP_EXT}", snapshot::now_millis()));
    let tmp = file.with_extension("tmp");
    snapshot::create(game_key, device_id, locations, &tmp)?;
    std::fs::rename(&tmp, &file)?;
    Ok(file)
}

/// Список валидных бэкапов в папке (битые файлы пропускаются),
/// свежие первыми. `game_key` — опциональный фильтр.
pub fn list_backups(dir: &Path, game_key: Option<&str>) -> io::Result<Vec<BackupInfo>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(BACKUP_EXT) {
            continue;
        }
        let Ok(m) = snapshot::read_manifest(&path) else {
            continue; // битый/чужой файл — пропускаем
        };
        if let Some(filter) = game_key {
            if m.game_key != filter {
                continue;
            }
        }
        out.push(BackupInfo {
            path,
            game_key: m.game_key,
            device_id: m.device_id,
            created_at: m.created_at,
            content_hash: m.content_hash,
        });
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Ok(out)
}

/// Ротация файловых бэкапов игры в папке: оставить `keep` свежих.
/// Возвращает число удалённых файлов.
pub fn prune_backup_files(dir: &Path, game_key: &str, keep: usize) -> io::Result<usize> {
    let backups = list_backups(dir, Some(game_key))?;
    let mut removed = 0;
    for b in backups.into_iter().skip(keep) {
        std::fs::remove_file(&b.path)?;
        removed += 1;
    }
    Ok(removed)
}

/// Ротация авто-бэкапов движка (`<state>/backups/<game>/<ts>/`):
/// оставить `keep` свежих каталогов. Имена каталогов — timestamp,
/// сортируются лексикографически.
pub fn prune_dir_backups(state_dir: &Path, game_key: &str, keep: usize) -> io::Result<usize> {
    let dir = state_dir
        .join("backups")
        .join(crate::storage_paths::game_component(game_key)?);
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.reverse(); // свежие (больший ts) первыми
    let mut removed = 0;
    for name in names.into_iter().skip(keep) {
        std::fs::remove_dir_all(dir.join(name))?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "savesync-test-backup-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_save(home: &Path, content: &str) -> Vec<SaveLocation> {
        let dir = home.join("game/saves");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("slot1.sav"), content).unwrap();
        let resolved = dir.to_string_lossy().replace('\\', "/");
        vec![SaveLocation {
            location: None,
            pattern: "<home>/game/saves".into(),
            resolved: resolved.clone(),
            hits: vec![PathBuf::from(resolved)],
        }]
    }

    #[test]
    fn create_list_and_prune() {
        let dir = fixture("create");
        let home = dir.join("home");
        let dest = dir.join("backups");

        let path = create_backup("test-game", "pc", &write_save(&home, "v1"), &dest).unwrap();
        assert!(path.extension().and_then(|e| e.to_str()) == Some(BACKUP_EXT));
        // Файл самодостаточен: verify + manifest.
        assert!(snapshot::verify(&path).unwrap());

        // Ещё два бэкапа с нарастающим ts.
        std::thread::sleep(std::time::Duration::from_millis(2));
        create_backup("test-game", "pc", &write_save(&home, "v2"), &dest).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        create_backup("test-game", "pc", &write_save(&home, "v3"), &dest).unwrap();

        let all = list_backups(&dest, None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].game_key, "test-game");
        assert!(all[0].created_at >= all[1].created_at);

        // Фильтр по игре.
        assert!(list_backups(&dest, Some("other-game")).unwrap().is_empty());

        // Ротация: оставить 2 свежих.
        assert_eq!(prune_backup_files(&dest, "test-game", 2).unwrap(), 1);
        assert_eq!(list_backups(&dest, None).unwrap().len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_dir_backups_keeps_newest() {
        let dir = fixture("dirs");
        let backups = dir.join("backups/test-game");
        for ts in ["100", "200", "300", "400"] {
            std::fs::create_dir_all(backups.join(ts)).unwrap();
        }
        assert_eq!(prune_dir_backups(&dir, "test-game", 2).unwrap(), 2);
        let left: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["300", "400"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_skips_garbage() {
        let dir = fixture("garbage");
        std::fs::write(dir.join("junk.savesync-backup"), b"not a zip").unwrap();
        std::fs::write(dir.join("note.txt"), b"hi").unwrap();
        assert!(list_backups(&dir, None).unwrap().is_empty());
        // Отсутствующая папка — пустой список.
        assert!(list_backups(&dir.join("nope"), None).unwrap().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
