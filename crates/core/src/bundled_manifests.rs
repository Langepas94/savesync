//! Curated manifest snapshot bundled with the application (CC BY-NC-SA 4.0).
use crate::manifest::{parse_manifest_str, ManifestDb};
pub fn load() -> ManifestDb {
    let sources = [
        include_str!("../manifests/ancestors-the-humankind-odyssey.yaml"),
        include_str!("../manifests/don-t-starve-together.yaml"),
        include_str!("../manifests/flatout-ultimate-carnage-collector-s-edition.yaml"),
        include_str!("../manifests/maneater.yaml"),
        include_str!("../manifests/metro-2033.yaml"),
        include_str!("../manifests/mudrunner.yaml"),
        include_str!("../manifests/no-man-s-sky.yaml"),
        include_str!("../manifests/scrap-mechanic.yaml"),
        include_str!("../manifests/surviving-mars.yaml"),
        include_str!("../manifests/teardown.yaml"),
        include_str!("../manifests/the-entropy-centre.yaml"),
        include_str!("../manifests/uncrashed-fpv-drone-simulator.yaml"),
        include_str!("../manifests/warhammer-40-000-dawn-of-war-ii-anniversary-edition.yaml"),
        include_str!("../manifests/warhammer-40-000-dawn-of-war-ii-chaos-rising.yaml"),
        include_str!("../manifests/warhammer-40-000-dawn-of-war-ii-retribution.yaml"),
        include_str!("../manifests/warhammer-40-000-space-marine-2-public-test-server.yaml"),
        include_str!("../manifests/warhammer-40-000-space-marine-2.yaml"),
    ];
    ManifestDb::from_entries(
        sources
            .into_iter()
            .map(|yaml| parse_manifest_str(yaml).expect("validated bundled manifest"))
            .collect(),
    )
}
#[cfg(test)]
mod tests {
    #[test]
    fn bundled_manifests_are_valid() {
        let database = super::load();
        assert!(!database.is_empty());
        for manifest in database.iter() {
            assert!(manifest.validate().is_empty(), "{}", manifest.slug);
        }
    }
}
