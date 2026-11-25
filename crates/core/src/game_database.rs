//! Offline game-save paths derived from the Ludusavi manifest.
use crate::game_key::slugify;
use crate::manifest::{Ids, Manifest, ManifestDb, Os, PathRule, Portability};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Game {
    name: String,
    ids: Vec<String>,
    aliases: Vec<String>,
    files: Vec<(String, Vec<String>)>,
}

fn database() -> &'static [Game] {
    static DATA: OnceLock<Vec<Game>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../manifests/ludusavi.json"))
            .expect("bundled game database is valid")
    })
}

fn convert(game: &Game, installed: &HashSet<u32>) -> Option<Manifest> {
    let mut paths = Vec::new();
    for (source, systems) in &game.files {
        let path = source
            .replace("<storeUserId>", "*")
            .replace("<osUserName>", "*")
            .replace("<winLocalAppData>Low", "<winLocalAppDataLow>")
            .replace("<winLocalAppData>low", "<winLocalAppDataLow>")
            .replace("<home>/AppData/LocalLow/", "<winLocalAppDataLow>/")
            .replace("<home>/Saved Games/", "<winSavedGames>/");
        let windows = systems.is_empty() || systems.iter().any(|os| os == "windows");
        let linux = systems.is_empty() || systems.iter().any(|os| os == "linux");
        let win = [
            ("<winDocuments>", "<documents>", "Documents"),
            ("<winAppData>", "<winAppData>", "AppData/Roaming"),
            ("<winLocalAppData>", "<winLocalAppData>", "AppData/Local"),
            (
                "<winLocalAppDataLow>",
                "<home>/AppData/LocalLow",
                "AppData/LocalLow",
            ),
            ("<winSavedGames>", "<winSavedGames>", "Saved Games"),
        ];
        if let Some((prefix, placeholder, proton)) =
            win.iter().find(|(p, _, _)| path.starts_with(p))
        {
            if !windows {
                continue;
            }
            let tail = &path[prefix.len()..];
            let normal = format!("{placeholder}{tail}");
            let compat = format!(
                "<steamCompat>/{}/pfx/drive_c/users/steamuser/{proton}{tail}",
                game.ids
                    .iter()
                    .find(|id| id.parse::<u32>().is_ok_and(|id| installed.contains(&id)))
                    .or_else(|| game.ids.first())
                    .map(String::as_str)
                    .unwrap_or("0")
            );
            if !normal.contains('<')
                || normal.starts_with(placeholder) && !normal[placeholder.len()..].contains('<')
            {
                paths.push(PathRule {
                    location: None,
                    pattern: normal,
                    os: Os::Windows,
                    tags: vec!["save".into()],
                });
                paths.push(PathRule {
                    location: None,
                    pattern: compat,
                    os: Os::Linux,
                    tags: vec!["save".into()],
                });
            }
        } else if let Some(tail) = path.strip_prefix("<root>/userdata/") {
            let Some((account, remaining)) = tail.split_once('/') else {
                continue;
            };
            let (id, relative) = remaining.split_once('/').unwrap_or((remaining, ""));
            if account != "*"
                || id.parse::<u32>().is_err()
                || relative.contains('<')
                || relative.split('/').any(|part| part == "..")
            {
                continue;
            }
            let relative = if relative.is_empty() {
                "remote"
            } else {
                relative
            };
            paths.push(PathRule {
                location: None,
                pattern: format!("<steamUserdata>/*/{id}/{relative}"),
                os: Os::Any,
                tags: vec!["save".into()],
            });
        } else if let Some(tail) = path.strip_prefix("<base>/") {
            if !tail.contains('<') && !tail.split('/').any(|part| part == "..") {
                for os in [Os::Windows, Os::Linux] {
                    if (os == Os::Windows && windows) || (os == Os::Linux && (linux || windows)) {
                        paths.push(PathRule {
                            location: None,
                            pattern: path.clone(),
                            os,
                            tags: vec!["save".into()],
                        });
                    }
                }
            }
        } else if linux
            && ["<home>/", "<xdgData>/", "<xdgConfig>/"]
                .iter()
                .any(|prefix| path.starts_with(prefix))
            && !path.split_once('>').unwrap().1.contains('<')
        {
            paths.push(PathRule {
                location: None,
                pattern: path,
                os: Os::Linux,
                tags: vec!["save".into()],
            });
        }
    }
    if paths.is_empty() {
        return None;
    }
    Some(Manifest {
        schema: 1,
        name: game.name.clone(),
        slug: slugify(&game.name),
        catalog_key: None,
        aliases: game.aliases.clone(),
        ids: Ids {
            steam: game.ids.clone(),
            ..Ids::default()
        },
        paths,
        registry: vec![],
        portability: Portability::default(),
    })
}

pub fn add_matching(db: &mut ManifestDb, ids: &HashSet<u32>, names: &HashSet<String>) {
    add_matching_where(db, ids, |name| names.contains(&slugify(name)));
}

pub(crate) fn add_installation_matches(db: &mut ManifestDb, names: &HashSet<String>) {
    use crate::game_key::title_initials;
    let short_names: HashSet<_> = names.iter().map(|name| name.replace('-', "")).collect();
    add_matching_where(db, &HashSet::new(), |name| {
        let slug = slugify(name);
        names.contains(&slug)
            || title_initials(&slug).is_some_and(|short| short_names.contains(&short))
    });
}

fn add_matching_where(db: &mut ManifestDb, ids: &HashSet<u32>, matches: impl Fn(&str) -> bool) {
    for game in database() {
        if !game
            .ids
            .iter()
            .any(|id| id.parse::<u32>().is_ok_and(|id| ids.contains(&id)))
            && !matches(&game.name)
            && !game.aliases.iter().any(|name| matches(name))
        {
            continue;
        }
        let Some(mut manifest) = convert(game, ids) else {
            continue;
        };
        if !manifest.validate().is_empty() {
            continue;
        }
        if let Some(existing) = db.get(&manifest.slug) {
            // Curated rules and their portability restrictions take precedence.
            let mut merged = existing.clone();
            for rule in manifest.paths.drain(..) {
                if !merged
                    .paths
                    .iter()
                    .any(|old| old.pattern == rule.pattern && old.os == rule.os)
                {
                    merged.paths.push(rule);
                }
            }
            for id in manifest.ids.steam {
                if !merged.ids.steam.contains(&id) {
                    merged.ids.steam.push(id);
                }
            }
            db.insert(merged);
        } else {
            db.insert(manifest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{Platform, Roots};

    #[test]
    fn bundled_database_finds_steam_game_without_ufs() {
        let tmp = tempfile::tempdir().unwrap();
        let save = tmp.path().join("steamapps/compatdata/12210/pfx/drive_c/users/steamuser/Documents/Rockstar Games/GTA IV/Profiles/123/SGTA400");
        std::fs::create_dir_all(save.parent().unwrap()).unwrap();
        std::fs::write(&save, b"save").unwrap();
        let mut db = ManifestDb::default();
        add_matching(&mut db, &HashSet::from([12210]), &HashSet::new());
        let roots = Roots {
            steam_libraries: vec![tmp.path().into()],
            ..Roots::default()
        };
        let found = crate::scanner::scan(&db, Platform::Linux, &roots);
        assert!(found.iter().any(|game| game
            .locations
            .iter()
            .any(|location| location.hits.iter().any(
                |hit| crate::paths::canonical_path(hit) == crate::paths::canonical_path(&save)
            ))));
        let before = db.get("grand-theft-auto-iv").unwrap().paths.len();
        add_matching(&mut db, &HashSet::from([12210]), &HashSet::new());
        assert_eq!(before, db.get("grand-theft-auto-iv").unwrap().paths.len());
    }
}
