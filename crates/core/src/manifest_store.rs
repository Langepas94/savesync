//! Validate a complete manifest update before replacing the managed database.
use crate::manifest::load_manifest_tree;
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
    let nonce = format!("{}-{}", std::process::id(), crate::snapshot::now_millis());
    let stage = parent.join(format!("manifests-stage-{nonce}"));
    let previous = parent.join(format!("manifests-previous-{nonce}"));
    std::fs::create_dir(&stage)?;
    let result = (|| {
        for manifest in database.iter() {
            std::fs::write(
                stage.join(format!(
                    "{}.yaml",
                    crate::storage_paths::game_component(&manifest.slug)?
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
mod tests {
    use super::*;
    #[test]
    fn invalid_update_preserves_installed_database_and_custom_rules() {
        let root = std::env::temp_dir().join(format!("ss-manifest-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("source/manifests")).unwrap();
        std::fs::create_dir_all(root.join("state/custom")).unwrap();
        std::fs::write(root.join("state/custom/mine.yaml"), "untouched").unwrap();
        std::fs::write(
            root.join("source/manifests/game.yaml"),
            "schema: 1\nname: Game\nslug: game\npaths:\n - pattern: '<home>/saves/*.sav'\n",
        )
        .unwrap();
        let destination = root.join("manifests");
        assert_eq!(install(&root.join("source"), &destination).unwrap(), 1);
        let previous = std::fs::read(destination.join("game.yaml")).unwrap();
        std::fs::write(root.join("source/manifests/broken.yaml"), "not a manifest").unwrap();
        assert!(install(&root.join("source"), &destination).is_err());
        assert_eq!(
            std::fs::read(destination.join("game.yaml")).unwrap(),
            previous
        );
        assert_eq!(
            std::fs::read_to_string(root.join("state/custom/mine.yaml")).unwrap(),
            "untouched"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
