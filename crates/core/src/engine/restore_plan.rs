//! Stage a verified archive and identify obsolete files tracked by prior applied state.
use super::file_transaction::{absolute_target, target_key, Transaction};
use super::*;

pub(super) struct Stage(pub PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl<B: Backend> Engine<B> {
    pub(super) fn matches_cached_version(
        &self,
        manifest: &Manifest,
        roots: &Roots,
        platform: Platform,
        id: &str,
        expected_hash: &str,
    ) -> bool {
        let Ok(path) = self.cache_path(&manifest.slug, id) else {
            return false;
        };
        if !snapshot::verify(&path).unwrap_or(false) {
            return false;
        }
        let Ok(source) = snapshot::read_manifest(&path) else {
            return false;
        };
        if source.game_key != manifest.slug
            || source.content_hash != expected_hash
            || source
                .locations
                .iter()
                .all(|location| location.files.is_empty())
        {
            return false;
        }
        for location in &source.locations {
            let Some((mapped, base)) = destinations::resolve(manifest, roots, platform, location)
            else {
                return false;
            };
            if snapshot::validate_destination(&mapped, &base).is_err() {
                return false;
            }
            for file in &mapped.files {
                let target = base.join(&file.rel_path);
                if !std::fs::metadata(&target)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() == file.size)
                {
                    return false;
                }
                if !crate::hash::hash_file(&target)
                    .is_ok_and(|hash| hash.to_hex().as_str() == file.hash)
                {
                    return false;
                }
            }
        }
        true
    }
    pub(super) fn prepare_restore(
        &self,
        manifest: &Manifest,
        roots: &Roots,
        platform: Platform,
        archive: &Path,
        sources: &snapshot::SnapshotManifest,
        targets: &[(snapshot::LocationManifest, PathBuf)],
    ) -> Result<(Transaction, Stage), EngineError> {
        let directory = self
            .state_dir
            .join("tmp")
            .join(format!("restore-{}", file_transaction::unique_stamp()));
        std::fs::create_dir(&directory)?;
        let stage = Stage(directory);
        let mut desired = Vec::new();
        let mut wanted = std::collections::HashSet::new();
        for (index, (location, base)) in targets.iter().enumerate() {
            let source = sources
                .locations
                .iter()
                .find(|source| source.archive_dir == location.archive_dir)
                .unwrap();
            let staged = stage.0.join(index.to_string());
            snapshot::extract_location_mapped(archive, source, location, &staged)?;
            for file in &location.files {
                let target = absolute_target(&base.join(&file.rel_path))?;
                wanted.insert(target_key(&target));
                desired.push((target, staged.join(&file.rel_path)));
            }
        }
        let mut removed = Vec::new();
        if let Some(previous) = self.applied_of(&manifest.slug) {
            let path = self.cache_path(&manifest.slug, &previous)?;
            if path.is_file() && snapshot::verify(&path).unwrap_or(false) {
                let previous = snapshot::read_manifest(&path)?;
                for location in &previous.locations {
                    if let Some((mapped, base)) =
                        destinations::resolve(manifest, roots, platform, location)
                    {
                        snapshot::validate_destination(&mapped, &base)?;
                        for file in &mapped.files {
                            let target = absolute_target(&base.join(&file.rel_path))?;
                            if !wanted.contains(&target_key(&target)) && !removed.contains(&target)
                            {
                                removed.push(target);
                            }
                        }
                    }
                }
            }
        }
        let transaction = Transaction::prepare(&self.state_dir, &manifest.slug, desired, removed)?;
        Ok((transaction, stage))
    }
}
