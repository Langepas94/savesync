//! Resolve selected installation folders through the available game database.
use crate::{
    game_key::{slugify, title_initials},
    manifest::{Manifest, ManifestDb},
    paths::{Platform, Roots},
};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

mod fallback;
mod launchers;
mod prefixes;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Shortcut {
    app_id: String,
    name: String,
    options: String,
    executable: PathBuf,
    start_dir: Option<PathBuf>,
}

fn steam_shortcuts(roots: &Roots) -> BTreeSet<Shortcut> {
    use crate::platform::steam::kv_binary::{Parser, Value};

    let mut matches = BTreeSet::new();
    for root in &roots.steam_libraries {
        let pattern = format!(
            "{}/userdata/*/config/shortcuts.vdf",
            crate::platform::paths::literal_save_pattern(root)
        );
        let Ok(files) = glob::glob(&pattern) else {
            continue;
        };
        for file in files.flatten() {
            let Ok(data) = std::fs::read(file) else {
                continue;
            };
            let Ok(tree) = Parser::new(&data, None).map() else {
                continue;
            };
            let Some(entries) = tree
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("shortcuts"))
                .and_then(|(_, value)| value.as_map())
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
                        .find(|(field, _)| field.eq_ignore_ascii_case(key))
                        .map(|(_, value)| value)
                };
                let (Some(executable), Some(name), Some(Value::I32(id))) = (
                    get("exe").and_then(Value::as_str),
                    get("appname").and_then(Value::as_str),
                    get("appid"),
                ) else {
                    continue;
                };
                let executable = Path::new(executable.trim_matches('"'));
                if !executable.is_absolute() {
                    continue;
                }
                let start_dir = get("startdir")
                    .and_then(Value::as_str)
                    .map(|value| Path::new(value.trim_matches('"')))
                    .filter(|path| path.is_absolute() && path.is_dir())
                    .map(crate::platform::paths::canonical_path);
                matches.insert(Shortcut {
                    app_id: (*id as u32).to_string(),
                    name: name.to_string(),
                    options: get("launchoptions")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    executable: crate::platform::paths::canonical_path(executable),
                    start_dir,
                });
            }
        }
    }
    matches
}

pub fn discover_steam_shortcuts(
    db: &ManifestDb,
    roots: &Roots,
    platform: Platform,
) -> Vec<Manifest> {
    if platform != Platform::Linux {
        return Vec::new();
    }
    let mut discovered = Vec::new();
    for shortcut in steam_shortcuts(roots) {
        let Some(parent) = shortcut
            .executable
            .parent()
            .filter(|parent| parent.parent().is_some())
        else {
            continue;
        };
        if !shortcut.executable.is_file() {
            continue;
        }
        let folder = shortcut
            .start_dir
            .as_deref()
            .filter(|path| path.parent().is_some() && shortcut.executable.starts_with(path))
            .unwrap_or(parent);
        if folder.starts_with("/usr")
            || folder.starts_with("/bin")
            || roots.home.as_deref() == Some(folder)
        {
            continue;
        }
        if !shortcut
            .executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
            && shortcut.start_dir.as_deref() != Some(folder)
        {
            continue;
        }
        let Ok(custom) = Manifest::custom(
            &shortcut.name,
            vec![crate::catalog::manifest::PathRule {
                location: None,
                pattern: crate::platform::paths::literal_save_pattern(folder),
                os: crate::catalog::manifest::Os::Linux,
                tags: vec!["save".into()],
            }],
        ) else {
            continue;
        };
        let (mut manifest, identity) =
            resolve_folder(custom, db, roots, platform, folder, Some(&shortcut));
        if let Some(identity) = identity {
            manifest.slug = identity;
        }
        if !manifest.paths.is_empty() {
            discovered.push(manifest);
        }
    }
    discovered
}

pub fn resolve_custom(
    custom: &Manifest,
    db: &ManifestDb,
    roots: &Roots,
    platform: Platform,
) -> Manifest {
    resolve_custom_with_identity(custom, db, roots, platform).0
}

pub fn resolve_custom_with_identity(
    custom: &Manifest,
    db: &ManifestDb,
    roots: &Roots,
    platform: Platform,
) -> (Manifest, Option<String>) {
    let explicit = || {
        let mut manifest = custom.clone();
        if let Some(known) = custom.catalog_key.as_deref().and_then(|key| db.get(key)) {
            manifest.portability = known.portability.clone();
            manifest.ids = known.ids.clone();
        }
        (manifest, custom.catalog_key.clone())
    };
    if custom.paths.len() != 1 {
        return explicit();
    }
    let Ok(mut matches) = glob::glob(&custom.paths[0].pattern) else {
        return explicit();
    };
    let Some(Ok(folder)) = matches.next() else {
        return explicit();
    };
    if matches.next().is_some() || !folder.is_dir() {
        return explicit();
    }
    let folder = crate::platform::paths::canonical_path(&folder);
    let shortcuts = if platform == Platform::Linux {
        steam_shortcuts(roots)
    } else {
        BTreeSet::new()
    };
    let mut matching = shortcuts
        .iter()
        .filter(|entry| entry.executable.starts_with(&folder));
    let first = matching.next();
    let shortcut = if matching.next().is_none() {
        first
    } else {
        None
    };
    let resolved = resolve_folder(custom.clone(), db, roots, platform, &folder, shortcut);
    if resolved.1.is_none() && custom.catalog_key.is_some() {
        let (mut manifest, identity) = explicit();
        manifest.paths = resolved.0.paths;
        manifest.installed_at = resolved.0.installed_at;
        (manifest, identity)
    } else {
        resolved
    }
}

fn resolve_folder(
    mut result: Manifest,
    db: &ManifestDb,
    roots: &Roots,
    platform: Platform,
    folder: &Path,
    shortcut: Option<&Shortcut>,
) -> (Manifest, Option<String>) {
    let executables = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                && path.is_file()
        })
        .collect::<Vec<_>>();
    let folder_name = folder
        .file_name()
        .and_then(|name| name.to_str())
        .map(slugify);
    let is_installation = !executables.is_empty() || shortcut.is_some();
    if is_installation {
        result.installed_at = Some(folder.to_path_buf());
    }
    let names: Vec<_> = if is_installation {
        shortcut
            .iter()
            .map(|entry| slugify(&entry.name))
            .chain([slugify(&result.name)])
            .chain(folder_name)
            .collect()
    } else {
        folder_name.into_iter().collect()
    };
    let executable_names: HashSet<_> = executables
        .iter()
        .filter_map(|path| path.file_stem().and_then(|name| name.to_str()))
        .map(slugify)
        .filter(|name| {
            !matches!(
                name.as_str(),
                "game" | "launcher" | "start" | "run" | "launch" | "setup"
            )
        })
        .collect();
    let mut fallback_db = ManifestDb::default();
    crate::catalog::game_database::add_installation_matches(
        &mut fallback_db,
        &names.iter().chain(&executable_names).cloned().collect(),
    );
    let manifest = names
        .iter()
        .find_map(|name| match_name(db, &fallback_db, |candidate| candidate == name))
        .or_else(|| {
            match_name(db, &fallback_db, |candidate| {
                executable_names.contains(candidate)
            })
        })
        .or_else(|| {
            if !is_installation {
                return None;
            }
            let abbreviations: HashSet<_> = names
                .iter()
                .chain(&executable_names)
                .map(|name| name.replace('-', ""))
                .collect();
            match_name(db, &fallback_db, |candidate| {
                title_initials(candidate).is_some_and(|short| abbreviations.contains(&short))
            })
        });
    if manifest.is_none() && !is_installation {
        return (result, None);
    }
    let users = if platform == Platform::Linux {
        prefixes::windows_users(folder, roots, shortcut.map(|entry| entry.options.as_str()))
    } else {
        BTreeSet::new()
    };
    let Some(manifest) = manifest else {
        // A directory with an executable is not safe to back up as a save folder.
        result.paths = fallback::save_rules(folder, platform);
        let profiles = relocated_profile_rules(roots, &users, &result, platform);
        result.paths.extend(profiles);
        return (result, None);
    };
    result.paths = manifest.paths.clone();
    bind_installation_rules(&mut result, folder);
    result.portability = manifest.portability.clone();
    result.ids = manifest.ids.clone();
    if platform == Platform::Linux {
        prefixes::resolve(
            &mut result.paths,
            folder,
            roots,
            shortcut.map(|entry| entry.app_id.as_str()),
            &users,
        );
    }
    result
        .paths
        .extend(relocated_profile_rules(roots, &users, manifest, platform));
    if is_installation {
        result.paths.extend(fallback::save_rules(folder, platform));
    }
    (result, Some(manifest.slug.clone()))
}

fn relocated_profile_rules(
    roots: &Roots,
    users: &BTreeSet<PathBuf>,
    game: &Manifest,
    platform: Platform,
) -> Vec<crate::catalog::manifest::PathRule> {
    let windows_areas = [
        "AppData/Roaming",
        "AppData/Local",
        "AppData/LocalLow",
        "Application Data",
        "Saved Games",
        "Documents",
        "My Documents",
    ];
    let areas = users
        .iter()
        .flat_map(|user| windows_areas.map(|area| user.join(area)))
        .chain(
            [
                &roots.documents,
                &roots.win_appdata,
                &roots.win_local_appdata,
                &roots.win_saved_games,
                &roots.xdg_data,
                &roots.xdg_config,
            ]
            .into_iter()
            .flatten()
            .cloned(),
        )
        .collect::<Vec<_>>();
    fallback::profile_save_rules(areas.iter().map(PathBuf::as_path), game, platform)
}

pub fn relocated_installed_profiles(
    game: &Manifest,
    folder: &Path,
    roots: &Roots,
    platform: Platform,
) -> Vec<crate::catalog::manifest::PathRule> {
    let users = if platform == Platform::Linux {
        prefixes::windows_users(folder, roots, None)
    } else {
        BTreeSet::new()
    };
    relocated_profile_rules(roots, &users, game, platform)
}

pub(crate) fn bind_installation_rules(manifest: &mut Manifest, folder: &Path) {
    for rule in &mut manifest.paths {
        let relative = rule.pattern.strip_prefix("<base>/").or_else(|| {
            rule.pattern
                .strip_prefix("<steamInstall>/")?
                .split_once('/')
                .map(|(_, tail)| tail)
        });
        if let Some(tail) = relative {
            if !tail.split('/').any(|part| part == "..") {
                rule.location
                    .get_or_insert_with(|| format!("install/{tail}"));
                rule.pattern = format!(
                    "{}/{tail}",
                    crate::platform::paths::literal_save_pattern(folder)
                );
            }
        }
    }
}

fn match_name<'a>(
    db: &'a ManifestDb,
    fallback: &'a ManifestDb,
    matches: impl Fn(&str) -> bool,
) -> Option<&'a Manifest> {
    let mut known = db.iter().chain(fallback.iter()).filter(|manifest| {
        !manifest.slug.starts_with("custom:")
            && (matches(&manifest.slug)
                || matches(&slugify(&manifest.name))
                || manifest
                    .aliases
                    .iter()
                    .any(|alias| matches(&slugify(alias))))
    });
    let matched = known.next()?;
    (!known.any(|other| other.slug != matched.slug)).then_some(matched)
}

#[cfg(test)]
#[path = "../tests/platform/installation.rs"]
mod tests;
