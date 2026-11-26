//! Curated manifest snapshot bundled with the application (CC BY-NC-SA 4.0).
use crate::catalog::manifest::{Manifest, ManifestDb};
use serde::Deserialize;

pub fn load() -> ManifestDb {
    let catalog = include_str!("../../manifests/catalog.yaml");
    let mut db = ManifestDb::default();
    for entry in serde_yaml::Deserializer::from_str(catalog) {
        let manifest = Manifest::deserialize(entry).expect("invalid bundled manifest YAML");
        let errors = manifest.validate();
        assert!(errors.is_empty(), "{}: {errors:?}", manifest.slug);
        assert!(
            db.get(&manifest.slug).is_none(),
            "duplicate manifest: {}",
            manifest.slug
        );
        db.insert(manifest);
    }
    db
}

#[cfg(test)]
#[path = "../tests/catalog/bundled_manifests.rs"]
mod tests;
