//! Resolve selected installation folders through the available game database.
use crate::{
    game_key::slugify,
    manifest::{Manifest, ManifestDb, Os},
    paths::{Platform, Roots},
};
use std::collections::HashSet;
use std::path::Path;

// Match the executable path, not the user-provided SaveSync display name.
fn steam_shortcut(folder: &Path, roots: &Roots) -> Option<(String, String)> {
    use crate::steam::kv_binary::{Parser, Value};
    let folder = crate::paths::canonical_path(folder);
    let mut matches = HashSet::new();
    for root in &roots.steam_libraries {
        let pattern = format!(
            "{}/userdata/*/config/shortcuts.vdf",
            glob::Pattern::escape(&root.to_string_lossy().replace('\\', "/"))
        );
        for file in glob::glob(&pattern).ok()?.flatten() {
            let Ok(data) = std::fs::read(file) else {
                continue;
            };
            let Ok(tree) = Parser::new(&data, None).map() else {
                continue;
            };
            let Some(entries) = tree
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("shortcuts"))
                .and_then(|(_, v)| v.as_map())
            else {
                continue;
            };
            for (_, entry) in entries {
                let Some(fields) = entry.as_map() else {
                    continue;
                };
                let get = |key: &str| {
                    fields
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(key))
                        .map(|(_, v)| v)
                };
                let (Some(exe), Some(name), Some(Value::I32(id))) = (
                    get("exe").and_then(Value::as_str),
                    get("appname").and_then(Value::as_str),
                    get("appid"),
                ) else {
                    continue;
                };
                let exe = Path::new(exe.trim_matches('"'));
                if exe.is_absolute() && crate::paths::canonical_path(exe).starts_with(&folder) {
                    matches.insert((name.to_string(), (*id as u32).to_string()));
                }
            }
        }
    }
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

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
    let shortcut = if platform == Platform::Linux {
        steam_shortcut(&folder, roots)
    } else {
        None
    };
    let executable = std::fs::read_dir(&folder).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("exe"))
                && entry.path().is_file()
        })
    });
    if !executable && shortcut.is_none() {
        return result;
    }
    // Never treat a detected installation as a save folder, even when no match exists.
    result.paths.clear();
    let name = slugify(
        shortcut
            .as_ref()
            .map_or(custom.name.as_str(), |s| s.0.as_str()),
    );
    let mut shortcut_db = ManifestDb::default();
    crate::game_database::add_matching(
        &mut shortcut_db,
        &HashSet::new(),
        &HashSet::from([name.clone()]),
    );
    let mut known = db.iter().chain(shortcut_db.iter()).filter(|m| {
        !m.slug.starts_with("custom:")
            && (slugify(&m.name) == name || m.aliases.iter().any(|alias| slugify(alias) == name))
    });
    let Some(manifest) = known.next() else {
        return result;
    };
    if known.any(|other| other.slug != manifest.slug) {
        return result;
    }
    result.paths = manifest.paths.clone();
    for rule in &mut result.paths {
        if let Some(tail) = rule.pattern.strip_prefix("<base>/") {
            rule.pattern = format!("{}/{tail}", crate::paths::literal_save_pattern(&folder));
        }
    }
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
        if let Some(id) = shortcut.as_ref().map(|s| &s.1).or_else(|| {
            if prefixes.len() == 1 {
                prefixes.iter().next()
            } else {
                None
            }
        }) {
            for rule in result.paths.iter_mut().filter(|r| r.os == Os::Linux) {
                if let Some(rest) = rule.pattern.strip_prefix("<steamCompat>/") {
                    if let Some((_, tail)) = rest.split_once('/') {
                        rule.pattern = format!("<steamCompat>/{id}/{tail}");
                    }
                }
            }
        } else if prefixes.len() > 1 {
            result
                .paths
                .retain(|rule| rule.os != Os::Linux || !rule.pattern.starts_with("<steamCompat>/"));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{parse_manifest_str, PathRule};

    #[test]
    fn steam_shortcut_links_arbitrary_display_name_to_its_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Downloads/arbitrary location");
        let root = dir.path().join("steam");
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("game.exe");
        std::fs::write(&exe, b"binary").unwrap();
        let saves = root.join("steamapps/compatdata/2147483768/pfx/drive_c/users/steamuser/Documents/Rockstar Games/Red Dead Redemption 2/Profiles/player");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::write(saves.join("slot"), b"save").unwrap();
        let shortcut = root.join("userdata/1/config/shortcuts.vdf");
        std::fs::create_dir_all(shortcut.parent().unwrap()).unwrap();
        let mut data = b"\x00shortcuts\0\x000\0\x02appid\0".to_vec();
        data.extend_from_slice(&2147483768u32.to_le_bytes());
        data.extend_from_slice(b"\x01AppName\0Red Dead Redemption 2\0\x01Exe\0");
        data.extend_from_slice(exe.to_string_lossy().as_bytes());
        data.extend_from_slice(b"\0\x08\x08\x08");
        std::fs::write(shortcut, data).unwrap();
        let custom = Manifest::custom(
            "Anything at all",
            vec![PathRule {
                location: None,
                pattern: crate::paths::literal_save_pattern(&folder),
                os: Os::Linux,
                tags: vec![],
            }],
        )
        .unwrap();
        let roots = Roots {
            steam_libraries: vec![root],
            ..Roots::default()
        };
        let resolved = resolve_custom(&custom, &ManifestDb::default(), &roots, Platform::Linux);
        let found = crate::scanner::scan(
            &ManifestDb::from_entries(vec![resolved]),
            Platform::Linux,
            &roots,
        );
        assert_eq!(
            crate::paths::canonical_path(&found[0].locations[0].hits[0]),
            crate::paths::canonical_path(&saves.join("slot"))
        );
    }

    #[test]
    fn finds_database_saves_inside_selected_installation() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Downloads/game [test]");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("game.exe"), b"binary").unwrap();
        std::fs::write(folder.join("140.sav"), b"save").unwrap();
        let custom = Manifest::custom(
            "140",
            vec![PathRule {
                location: None,
                pattern: crate::paths::literal_save_pattern(&folder),
                os: Os::Linux,
                tags: vec![],
            }],
        )
        .unwrap();
        let mut db = ManifestDb::default();
        crate::game_database::add_matching(
            &mut db,
            &HashSet::new(),
            &HashSet::from(["140".to_string()]),
        );
        let resolved = resolve_custom(&custom, &db, &Roots::default(), Platform::Linux);
        let found = crate::scanner::scan(
            &ManifestDb::from_entries(vec![resolved]),
            Platform::Linux,
            &Roots::default(),
        );
        assert_eq!(
            crate::paths::canonical_path(&found[0].locations[0].hits[0]),
            crate::paths::canonical_path(&folder.join("140.sav"))
        );
    }

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
        assert_eq!(
            crate::paths::canonical_path(&found[0].locations[0].hits[0]),
            crate::paths::canonical_path(&saves.join("slot.dat"))
        );
        let unknown = resolve_custom(&custom, &ManifestDb::default(), &roots, Platform::Linux);
        assert!(unknown.paths.is_empty());
    }
}
