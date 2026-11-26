use crate::game_key::{slugify, title_initials};
use crate::manifest::{Manifest, Os, PathRule};
use crate::paths::{literal_save_pattern, Platform};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

/// Look for recognisable save files only inside the selected installation.
pub(super) fn save_rules(folder: &Path, platform: Platform) -> Vec<PathRule> {
    let mut rules = Vec::new();
    let mut pending = vec![(folder.to_path_buf(), false, 0)];
    let os = match platform {
        Platform::Windows => Os::Windows,
        Platform::Linux => Os::Linux,
    };
    while let Some((directory, save_tree, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() && depth < 12 {
                let profile = is_profile_directory(&entry.file_name().to_string_lossy());
                pending.push((path, save_tree || profile, depth + 1));
            } else if kind.is_file()
                && (is_save_file(&path, save_tree || depth == 0)
                    || (save_tree && is_numbered_slot(&path)))
            {
                let Ok(relative) = path.strip_prefix(folder) else {
                    continue;
                };
                rules.push(PathRule {
                    location: Some(format!(
                        "install-file/{}",
                        relative.to_string_lossy().replace('\\', "/")
                    )),
                    pattern: literal_save_pattern(&path),
                    os,
                    tags: vec!["save".into()],
                });
            }
        }
    }
    rules.sort_by(|left, right| left.pattern.cmp(&right.pattern));
    rules
}

/// Discover relocated saves below folders that identify the selected title.
pub(super) fn profile_save_rules<'a>(
    roots: impl IntoIterator<Item = &'a Path>,
    game: &Manifest,
    platform: Platform,
) -> Vec<PathRule> {
    let mut names = HashSet::new();
    for name in std::iter::once(&game.name).chain(game.aliases.iter()) {
        let name = slugify(name);
        if let Some(initials) = title_initials(&name) {
            names.insert(initials);
        }
        let compact = name.replace('-', "");
        if compact.len() >= 3 && !matches!(compact.as_str(), "game" | "games" | "launcher") {
            names.insert(compact);
        }
    }
    names.extend(game.ids.steam.iter().filter(|id| id.len() >= 4).cloned());
    if names.is_empty() {
        return Vec::new();
    }

    let mut games = BTreeSet::new();
    let roots: BTreeSet<_> = roots
        .into_iter()
        .filter(|root| root.is_dir())
        .map(crate::paths::canonical_path)
        .collect();
    for root in &roots {
        let mut directories = vec![(root.clone(), 0)];
        while let Some((directory, depth)) = directories.pop() {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            {
                let name = entry.file_name();
                let key = slugify(&name.to_string_lossy()).replace('-', "");
                if names.contains(&key) {
                    games.insert(entry.path());
                } else if depth < 3
                    && !matches!(
                        key.as_str(),
                        "cache"
                            | "shadercache"
                            | "codecache"
                            | "temp"
                            | "logs"
                            | "steam"
                            | "steamapps"
                    )
                {
                    directories.push((entry.path(), depth + 1));
                }
            }
        }
    }

    let os = match platform {
        Platform::Windows => Os::Windows,
        Platform::Linux => Os::Linux,
    };
    let mut rules = Vec::new();
    for directory in games {
        let mut profiles = vec![(directory.clone(), 0)];
        while let Some((profile, depth)) = profiles.pop() {
            let Ok(entries) = std::fs::read_dir(&profile) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if kind.is_dir() && depth < 6 {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if is_profile_directory(&name) || is_profile_id(&name) {
                        profiles.push((path, depth + 1));
                    }
                } else if kind.is_file()
                    && (is_save_file(&path, true) || (depth > 0 && is_numbered_slot(&path)))
                {
                    let Ok(relative) = path.strip_prefix(&directory) else {
                        continue;
                    };
                    rules.push(PathRule {
                        location: Some(format!(
                            "profile-file/{}",
                            relative
                                .to_string_lossy()
                                .replace('\\', "/")
                                .to_ascii_lowercase()
                        )),
                        pattern: literal_save_pattern(&path),
                        os,
                        tags: vec!["save".into()],
                    });
                }
            }
        }
    }
    rules.sort_by(|left, right| left.pattern.cmp(&right.pattern));
    rules
}

fn is_profile_directory(name: &str) -> bool {
    matches!(
        slugify(name).replace('-', "").as_str(),
        "save"
            | "saves"
            | "savegames"
            | "savegame"
            | "savedata"
            | "gamesaves"
            | "profile"
            | "profiles"
    )
}

fn is_profile_id(name: &str) -> bool {
    name.len() >= 4
        && name.bytes().any(|byte| byte.is_ascii_digit())
        && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_numbered_slot(path: &Path) -> bool {
    if path.extension().is_some() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(first_digit) = name.bytes().position(|byte| byte.is_ascii_digit()) else {
        return false;
    };
    let (prefix, slot) = name.split_at(first_digit);
    (2..=12).contains(&prefix.len())
        && prefix.bytes().all(|byte| byte.is_ascii_alphabetic())
        && slot.len() >= 3
        && slot.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_save_file(path: &Path, save_directory: bool) -> bool {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "sav" | "save" | "savegame") {
        return true;
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    save_directory
        && matches!(extension.as_str(), "" | "dat" | "bin" | "json")
        && ["savegame", "quicksave", "autosave", "slot"]
            .iter()
            .any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'_' | b'-'))
                })
            })
}
