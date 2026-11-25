//! Hash-verified immutable publication cache. Network listings still determine membership.
use std::{cell::RefCell, collections::HashMap, io, path::PathBuf};
#[derive(Default)]
pub(crate) struct Cache {
    pub directory: Option<PathBuf>,
    memory: RefCell<HashMap<String, Vec<u8>>>,
}
impl Cache {
    pub fn persistent() -> Self {
        Self {
            directory: crate::config::data_dir(crate::paths::Platform::current())
                .map(|directory| directory.join("state/cloud-publications")),
            ..Self::default()
        }
    }
    pub fn get(
        &self,
        name: &str,
        fetch: impl FnOnce() -> io::Result<Vec<u8>>,
    ) -> io::Result<Vec<u8>> {
        let hash = name
            .strip_prefix(super::index_journal::PREFIX)
            .and_then(|name| name.strip_suffix(".json"))
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| io::Error::other("invalid publication object name"))?;
        if let Some(bytes) = self.memory.borrow().get(name) {
            return Ok(bytes.clone());
        }
        let path = self
            .directory
            .as_ref()
            .map(|directory| directory.join(format!("{hash}.json")));
        let cached = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .filter(|bytes| blake3::hash(bytes).to_hex().as_str() == hash);
        let bytes = match cached {
            Some(bytes) => bytes,
            None => {
                let bytes = fetch()?;
                if bytes.len() > 32 * 1024 * 1024 || blake3::hash(&bytes).to_hex().as_str() != hash
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "publication checksum/size mismatch",
                    ));
                }
                if let Some(path) = path {
                    if let Some(directory) = path.parent() {
                        if std::fs::create_dir_all(directory).is_ok() {
                            let temporary = directory.join(format!(
                                "{hash}.{}.tmp",
                                blake3::hash(oauth2::CsrfToken::new_random().secret().as_bytes())
                                    .to_hex()
                            ));
                            if std::fs::write(&temporary, &bytes).is_ok() {
                                let _ = std::fs::rename(&temporary, path);
                                let _ = std::fs::remove_file(temporary);
                            }
                        }
                    }
                }
                bytes
            }
        };
        let mut memory = self.memory.borrow_mut();
        if memory.values().map(Vec::len).sum::<usize>() + bytes.len() > 32 * 1024 * 1024 {
            memory.clear();
        }
        memory.insert(name.into(), bytes.clone());
        Ok(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_disk_cache_avoids_download_and_repairs_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"immutable publication".to_vec();
        let hash = blake3::hash(&bytes).to_hex();
        let name = format!("{}{hash}.json", crate::index_journal::PREFIX);
        let cache = Cache {
            directory: Some(directory.path().into()),
            ..Cache::default()
        };
        assert_eq!(cache.get(&name, || Ok(bytes.clone())).unwrap(), bytes);
        let reopened = Cache {
            directory: cache.directory.clone(),
            ..Cache::default()
        };
        assert_eq!(
            reopened
                .get(&name, || panic!("must use verified disk cache"))
                .unwrap(),
            bytes
        );
        std::fs::write(directory.path().join(format!("{hash}.json")), b"corrupt").unwrap();
        let reopened = Cache {
            directory: cache.directory.clone(),
            ..Cache::default()
        };
        assert_eq!(reopened.get(&name, || Ok(bytes.clone())).unwrap(), bytes);
    }
}
