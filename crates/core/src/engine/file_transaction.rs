//! Recoverable file replacement: durable backups, prepared writes and a journal.
use super::*;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Serialize, Deserialize)]
struct Entry {
    target: PathBuf,
    backup: Option<PathBuf>,
    prepared: Option<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    committed: bool,
    entries: Vec<Entry>,
    index: Option<Vec<u8>>,
    applied: Option<Vec<u8>>,
}

pub(super) struct Transaction {
    state: PathBuf,
    journal: Journal,
    active: bool,
    finished: bool,
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("transaction.tmp");
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    sync_parents(path)
}

fn copy_new(source: &Path, target: &Path) -> io::Result<()> {
    let mut input = std::fs::File::open(source)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let result = io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .and_then(|_| sync_parents(target));
    drop(output);
    if result.is_err() {
        let _ = std::fs::remove_file(target);
    }
    result
}

impl Transaction {
    pub(super) fn prepare(
        state: &Path,
        game: &str,
        desired: Vec<(PathBuf, PathBuf)>,
        removed: Vec<PathBuf>,
    ) -> io::Result<Self> {
        let component = crate::storage_paths::game_component(game)?;
        let stamp = unique_stamp();
        let backup_dir = state.join("backups").join(component).join(&stamp);
        let mut transaction = Self {
            state: state.into(),
            journal: Journal {
                committed: false,
                entries: vec![],
                index: read_optional(&state.join("index.json"))?,
                applied: read_optional(&state.join("applied.json"))?,
            },
            active: false,
            finished: false,
        };
        let mut names = std::collections::HashSet::new();
        for (target, source) in desired
            .into_iter()
            .map(|(target, source)| (target, Some(source)))
            .chain(removed.into_iter().map(|target| (target, None)))
        {
            let target = absolute_target(&target)?;
            if !names.insert(target_key(&target)) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate transaction target",
                ));
            }
            snapshot::validate_target_path(&target)?;
            let backup = match std::fs::metadata(&target) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.permissions().readonly() {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "restore target is not a writable file",
                        ));
                    }
                    let backup = backup_dir
                        .join(transaction.journal.entries.len().to_string())
                        .join(target.file_name().unwrap());
                    std::fs::create_dir_all(backup.parent().unwrap())?;
                    copy_new(&target, &backup)?;
                    Some(backup)
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let prepared = if source.is_some() {
                let parent = target.parent().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "restore target needs a parent")
                })?;
                std::fs::create_dir_all(parent)?;
                Some(parent.join(format!(
                    ".savesync-tx-{stamp}-{}.tmp",
                    transaction.journal.entries.len()
                )))
            } else {
                None
            };
            transaction.journal.entries.push(Entry {
                target,
                backup,
                prepared: None,
            });
            if let Some(source) = source {
                copy_new(&source, prepared.as_ref().unwrap())?;
                transaction.journal.entries.last_mut().unwrap().prepared = prepared;
            }
        }
        Ok(transaction)
    }

    fn save_journal(&self) -> io::Result<()> {
        atomic_bytes(
            &self.state.join("restore-journal.json"),
            &serde_json::to_vec(&self.journal)?,
        )
    }

    fn apply(&mut self) -> io::Result<()> {
        self.save_journal()?;
        self.active = true;
        for entry in &self.journal.entries {
            snapshot::validate_target_path(&entry.target)?;
            if let Some(prepared) = &entry.prepared {
                std::fs::rename(prepared, &entry.target)?;
            } else if entry.target.exists() {
                std::fs::remove_file(&entry.target)?;
            }
            sync_parents(&entry.target)?;
        }
        Ok(())
    }

    pub(super) fn commit<T>(
        mut self,
        metadata: impl FnOnce() -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let result = self
            .apply()
            .map_err(EngineError::from)
            .and_then(|_| metadata());
        match result {
            Ok(value) => {
                // Metadata is durable before the journal changes to committed.
                for name in ["index.json", "applied.json"] {
                    if self.state.join(name).is_file() {
                        std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(self.state.join(name))?
                            .sync_all()?;
                        sync_parents(&self.state.join(name))?;
                    }
                }
                self.journal.committed = true;
                if let Err(error) = self.save_journal() {
                    self.journal.committed = false;
                    return Err(error.into());
                }
                self.finished = true;
                let _ = std::fs::remove_file(self.state.join("restore-journal.json"));
                Ok(value)
            }
            Err(error) => {
                if self.active {
                    if let Err(rollback) = self.rollback() {
                        return Err(io::Error::other(format!("restore failed: {error}; rollback incomplete: {rollback}; safety backups and restore-journal.json retained")).into());
                    }
                }
                Err(error)
            }
        }
    }

    fn rollback(&mut self) -> io::Result<()> {
        for entry in &self.journal.entries {
            snapshot::validate_target_path(&entry.target)?;
        }
        for entry in self.journal.entries.iter().rev() {
            if let Some(backup) = &entry.backup {
                let temporary = entry
                    .target
                    .parent()
                    .unwrap()
                    .join(format!(".savesync-rollback-{}.tmp", unique_stamp()));
                copy_new(backup, &temporary)?;
                std::fs::rename(temporary, &entry.target)?;
            } else if entry.target.is_file() {
                std::fs::remove_file(&entry.target)?;
            }
            sync_parents(&entry.target)?;
        }
        for (name, bytes) in [
            ("index.json", &self.journal.index),
            ("applied.json", &self.journal.applied),
        ] {
            let target = self.state.join(name);
            if let Some(bytes) = bytes {
                atomic_bytes(&target, bytes)?;
            } else if target.exists() {
                std::fs::remove_file(&target)?;
                sync_parents(&target)?;
            }
        }
        std::fs::remove_file(self.state.join("restore-journal.json"))?;
        sync_parents(&self.state.join("restore-journal.json"))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if self.active && !self.finished && !self.journal.committed {
            let _ = self.rollback();
        }
        for entry in &self.journal.entries {
            if let Some(prepared) = &entry.prepared {
                let _ = std::fs::remove_file(prepared);
            }
        }
    }
}

pub(super) fn recover_pending(state: &Path) -> io::Result<()> {
    let Some(bytes) = read_optional(&state.join("restore-journal.json"))? else {
        return Ok(());
    };
    let journal: Journal = serde_json::from_slice(&bytes)?;
    // Reject paths outside our managed backup area before recovery reads them.
    for entry in &journal.entries {
        if entry
            .backup
            .as_ref()
            .is_some_and(|path| !path.starts_with(state.join("backups")))
            || entry.prepared.as_ref().is_some_and(|path| {
                path.parent() != entry.target.parent()
                    || !path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .starts_with(".savesync-tx-")
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid restore journal",
            ));
        }
    }
    let mut transaction = Transaction {
        state: state.into(),
        journal,
        active: true,
        finished: false,
    };
    if transaction.journal.committed {
        std::fs::remove_file(state.join("restore-journal.json"))?;
        transaction.finished = true;
    } else {
        transaction.rollback()?;
    }
    Ok(())
}

pub(super) fn unique_stamp() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{nanos:020}-{}-{:020}",
        nanos / 1_000_000,
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
pub(super) fn absolute_target(path: &Path) -> io::Result<PathBuf> {
    snapshot::validate_target_path(path)?;
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(crate::paths::canonical_path(&path))
}
pub(super) fn target_key(path: &Path) -> String {
    let key = path.to_string_lossy().to_string();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}
pub(super) fn state_guard(state: &Path) -> io::Result<std::fs::File> {
    std::fs::create_dir_all(state)?;
    let guard = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state.join("engine.lock"))?;
    fs2::FileExt::lock_exclusive(&guard)?;
    recover_pending(state)?;
    Ok(guard)
}

#[cfg(test)]
mod tests;

// Unix needs directory barriers in addition to file fsync for rename/remove
// ordering. Windows file FlushFileBuffers is used above; physical power-loss
// guarantees remain filesystem/hardware dependent and are not claimed by tests.
fn sync_parents(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    for directory in path.parent().into_iter().flat_map(Path::ancestors) {
        std::fs::File::open(directory)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
