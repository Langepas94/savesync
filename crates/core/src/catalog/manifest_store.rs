//! Validate a complete manifest update before replacing the managed database.
use crate::catalog::manifest::load_manifest_tree;
use std::{io, path::Path};

pub fn install(source: &Path, destination: &Path) -> io::Result<usize> {
    let manifest_root = source.join("manifests");
    let source = if manifest_root.is_dir() {
        manifest_root.as_path()
    } else {
        source
    };
    let (database, errors) = load_manifest_tree(source);
    if !errors.is_empty() || database.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid manifest database: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ));
    }
    let mut slugs = std::collections::HashSet::new();
    for manifest in database.iter() {
        if !manifest.validate().is_empty()
            || manifest.slug.starts_with("custom:")
            || !slugs.insert(manifest.slug.clone())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid or duplicate manifest: {}", manifest.slug),
            ));
        }
    }
    if std::fs::symlink_metadata(destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "manifest destination is a symlink",
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "manifest destination needs a parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        crate::storage::snapshot::now_millis()
    );
    let stage = parent.join(format!("manifests-stage-{nonce}"));
    let previous = parent.join(format!("manifests-previous-{nonce}"));
    std::fs::create_dir(&stage)?;
    let result = (|| {
        for manifest in database.iter() {
            std::fs::write(
                stage.join(format!(
                    "{}.yaml",
                    crate::storage::storage_paths::game_component(&manifest.slug)?
                )),
                manifest.to_yaml().map_err(io::Error::other)?,
            )?;
        }
        let had_previous = destination.exists();
        if had_previous {
            std::fs::rename(destination, &previous)?;
        }
        if let Err(error) = std::fs::rename(&stage, destination) {
            if had_previous {
                std::fs::rename(&previous, destination)?;
            }
            return Err(error);
        }
        if had_previous {
            std::fs::remove_dir_all(&previous)?;
        }
        Ok(database.len())
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

#[cfg(test)]
#[path = "../tests/catalog/manifest_store.rs"]
mod tests;
