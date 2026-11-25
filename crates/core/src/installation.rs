//! Resolve selected installation folders through the available game database.
use crate::{
    game_key::slugify,
    manifest::{Manifest, ManifestDb, Os},
    paths::{Platform, Roots},
};
use std::collections::HashSet;

pub fn resolve_custom(
    custom: &Manifest,
    db: &ManifestDb,
    roots: &Roots,
    platform: Platform,
) -> Manifest {
    let mut result = custom.clone();
    if custom.paths.len() != 1 {
        return result;
    }
    let Ok(mut matches) = glob::glob(&custom.paths[0].pattern) else {
        return result;
    };
    let Some(Ok(folder)) = matches.next() else {
        return result;
    };
    if matches.next().is_some() || !folder.is_dir() {
        return result;
    }
    let executable = std::fs::read_dir(&folder).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("exe"))
                && entry.path().is_file()
        })
    });
    if !executable {
        return result;
    }
    // Never treat a detected installation as a save folder, even when no match exists.
    result.paths.clear();
    let name = slugify(&custom.name);
    let mut known = db.iter().filter(|m| {
        !m.slug.starts_with("custom:")
            && (slugify(&m.name) == name || m.aliases.iter().any(|alias| slugify(alias) == name))
    });
    let Some(manifest) = known.next() else {
        return result;
    };
    if known.next().is_some() {
        return result;
    }
    result.paths = manifest.paths.clone();
    result.portability = manifest.portability.clone();
    if platform == Platform::Linux {
        let mut prefixes = HashSet::new();
        for rule in result.paths.iter().filter(|r| r.os == Os::Linux) {
            let Some(rest) = rule.pattern.strip_prefix("<steamCompat>/") else {
                continue;
            };
            let Some((_, tail)) = rest.split_once('/') else {
                continue;
            };
            for root in &roots.steam_libraries {
                let pattern = format!(
                    "{}/steamapps/compatdata/*/{tail}",
                    glob::Pattern::escape(&root.to_string_lossy().replace('\\', "/"))
                );
                if let Ok(paths) = glob::glob(&pattern) {
                    let compat = root.join("steamapps/compatdata");
                    for path in paths.flatten() {
                        if let Ok(relative) = path.strip_prefix(&compat) {
                            if let Some(id) = relative.components().next() {
                                prefixes.insert(id.as_os_str().to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
        if prefixes.len() == 1 {
            let id = prefixes.iter().next().unwrap();
            for rule in result.paths.iter_mut().filter(|r| r.os == Os::Linux) {
                if let Some(rest) = rule.pattern.strip_prefix("<steamCompat>/") {
                    if let Some((_, tail)) = rest.split_once('/') {
                        rule.pattern = format!("<steamCompat>/{id}/{tail}");
                    }
                }
            }
        } else if prefixes.len() > 1 {
            result.paths.retain(|rule| rule.os != Os::Linux);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{parse_manifest_str, PathRule};

    #[test]
    fn finds_saves_outside_arbitrary_installation_in_non_steam_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Downloads/anywhere");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("game.exe"), b"binary").unwrap();
        let saves = dir.path().join("steam/steamapps/compatdata/888/pfx/drive_c/users/steamuser/Documents/Studio/Game/saves");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::write(saves.join("slot.dat"), b"save").unwrap();
        let known = parse_manifest_str("schema: 1\nname: Example Game\nslug: example-game\npaths:\n - pattern: '<steamCompat>/123/pfx/drive_c/users/steamuser/Documents/Studio/Game/saves'\n   os: linux\n").unwrap();
        let custom = Manifest::custom(
            "Example Game",
            vec![PathRule {
                location: None,
                pattern: crate::paths::literal_save_pattern(&folder),
                os: Os::Linux,
                tags: vec![],
            }],
        )
        .unwrap();
        let roots = Roots {
            steam_libraries: vec![dir.path().join("steam")],
            ..Roots::default()
        };
        let resolved = resolve_custom(
            &custom,
            &ManifestDb::from_entries(vec![known]),
            &roots,
            Platform::Linux,
        );
        let found = crate::scanner::scan(
            &ManifestDb::from_entries(vec![resolved]),
            Platform::Linux,
            &roots,
        );
        assert_eq!(found[0].locations[0].hits, vec![saves.join("slot.dat")]);
        let unknown = resolve_custom(&custom, &ManifestDb::default(), &roots, Platform::Linux);
        assert!(unknown.paths.is_empty());
    }
}
