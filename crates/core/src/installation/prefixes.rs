use crate::manifest::{Os, PathRule};
use crate::paths::{canonical_path, literal_save_pattern, Roots};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub(super) fn resolve(
    rules: &mut Vec<PathRule>,
    folder: &Path,
    roots: &Roots,
    shortcut: Option<(&str, &str)>,
) {
    if !rules.iter().any(|rule| linux_prefix_tail(rule).is_some()) {
        return;
    }
    for rule in rules.iter_mut() {
        let identity = crate::paths::windows_save_pattern(&rule.pattern);
        if matches!(rule.os, Os::Windows) || identity.as_ref() != rule.pattern {
            rule.location
                .get_or_insert_with(|| format!("windows/{identity}"));
        }
    }
    let prefixes = find_prefixes(folder, roots, shortcut.map(|(_, options)| options));
    let steam_prefixes: Vec<_> = roots
        .steam_libraries
        .iter()
        .flat_map(|root| {
            directories(root, "steamapps/compatdata/*").filter_map(|path| {
                let id = path.file_name()?.to_str()?.to_string();
                Some((id, path))
            })
        })
        .collect();
    let users: BTreeSet<_> = prefixes
        .iter()
        .flat_map(|prefix| directories(prefix, "drive_c/users/*"))
        .collect();
    let installation_dirs = installation_directories(folder);
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();
    for rule in rules.drain(..) {
        let Some(tail) = linux_prefix_tail(&rule) else {
            expanded.push(rule);
            continue;
        };
        let Some((_, user_tail)) = tail
            .strip_prefix("pfx/drive_c/users/")
            .and_then(|rest| rest.split_once('/'))
        else {
            expanded.push(rule);
            continue;
        };
        let mut patterns = BTreeSet::new();
        let mut steam_patterns = BTreeMap::new();
        for (id, prefix) in &steam_prefixes {
            let path = format!("{}/{tail}", literal_save_pattern(prefix));
            let canonical = crate::paths::canonical_pattern(&path);
            if glob::glob(&canonical).is_ok_and(|mut hits| hits.next().is_some()) {
                steam_patterns.insert(canonical, format!("<steamCompat>/{id}/{tail}"));
            }
        }
        for user in &users {
            patterns.extend(existing_patterns(user, user_tail));
            if let Some(relative) = user_tail.strip_prefix("Documents/") {
                patterns.extend(existing_patterns(user, &format!("My Documents/{relative}")));
            }
        }
        if let (Some(documents), Some(relative)) =
            (&roots.documents, user_tail.strip_prefix("Documents/"))
        {
            patterns.extend(existing_patterns(documents, relative));
        }
        let portable_patterns = portable_patterns(folder, &installation_dirs, user_tail);
        if patterns.is_empty() {
            let mut fallback = rule.clone();
            if let Some((id, _)) = shortcut {
                fallback.pattern = format!("<steamCompat>/{id}/{tail}");
            }
            expanded.push(fallback);
        } else {
            for pattern in patterns {
                if seen.insert(pattern.clone()) {
                    let symbolic = steam_patterns.remove(&pattern).unwrap_or(pattern);
                    expanded.push(PathRule {
                        pattern: symbolic,
                        ..rule.clone()
                    });
                }
            }
        }
        for pattern in portable_patterns {
            if seen.insert(pattern.clone()) {
                expanded.push(PathRule {
                    pattern,
                    ..rule.clone()
                });
            }
        }
    }
    let mut patterns = HashSet::new();
    expanded
        .retain(|rule| linux_prefix_tail(rule).is_none() || patterns.insert(rule.pattern.clone()));
    *rules = expanded;
}

fn installation_directories(folder: &Path) -> BTreeMap<String, Vec<PathBuf>> {
    let mut directories = BTreeMap::<_, Vec<_>>::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(path) = pending.pop() {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            directories
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(path.clone());
        }
        if let Ok(entries) = std::fs::read_dir(&path) {
            pending.extend(
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| entry.path()),
            );
        }
    }
    directories
}

fn portable_patterns(
    folder: &Path,
    directories: &BTreeMap<String, Vec<PathBuf>>,
    user_tail: &str,
) -> BTreeSet<String> {
    let relative = [
        "Documents/",
        "My Documents/",
        "Saved Games/",
        "AppData/Local/",
        "AppData/Roaming/",
        "AppData/LocalLow/",
    ]
    .into_iter()
    .find_map(|scope| user_tail.strip_prefix(scope))
    .unwrap_or(user_tail);
    let mut patterns = BTreeSet::new();
    let (parent, tail) = split_static_parent(relative);
    if parent.is_empty() {
        patterns.extend(existing_patterns(folder, tail));
    } else if let Some(anchor) = parent.trim_end_matches('/').rsplit('/').next() {
        if let Some(paths) = directories.get(&anchor.to_ascii_lowercase()) {
            for path in paths {
                patterns.extend(existing_patterns(path, tail));
            }
        }
    }
    patterns
}

fn linux_prefix_tail(rule: &PathRule) -> Option<&str> {
    if rule.os == Os::Windows {
        return None;
    }
    rule.pattern
        .strip_prefix("<steamCompat>/")?
        .split_once('/')
        .map(|(_, tail)| tail)
}

fn find_prefixes(folder: &Path, roots: &Roots, launch_options: Option<&str>) -> BTreeSet<PathBuf> {
    let mut prefixes = BTreeSet::new();
    for library in &roots.steam_libraries {
        prefixes.extend(directories(library, "steamapps/compatdata/*/pfx"));
    }
    for tail in ["pfx", "*/pfx", "*"] {
        prefixes.extend(directories(folder, tail));
    }
    prefixes.insert(folder.to_path_buf());
    if let Some(home) = &roots.home {
        for tail in [
            ".wine",
            "Games/*",
            "Games/Heroic/Prefixes/*",
            "Games/Heroic/Prefixes/default/*",
            ".var/app/com.usebottles.bottles/data/bottles/bottles/*",
            ".var/app/com.heroicgameslauncher.hgl/config/heroic/Prefixes/*",
        ] {
            prefixes.extend(directories(home, tail));
        }
    }
    if let Some(data) = &roots.xdg_data {
        prefixes.extend(directories(data, "bottles/bottles/*"));
    }
    for (variable, proton) in [("WINEPREFIX", false), ("STEAM_COMPAT_DATA_PATH", true)] {
        let configured = launch_options
            .and_then(|options| launch_path(options, variable))
            .map(PathBuf::from);
        for path in configured
            .into_iter()
            .chain(std::env::var_os(variable).map(PathBuf::from))
        {
            if path.is_absolute() {
                prefixes.insert(if proton { path.join("pfx") } else { path });
            }
        }
    }
    for configured in super::launchers::game_prefixes(folder, roots) {
        prefixes.insert(configured.join("pfx"));
        prefixes.insert(configured);
    }
    for ancestor in folder.ancestors().take(6) {
        if ancestor.join("drive_c").is_dir() {
            prefixes.insert(ancestor.to_path_buf());
        }
    }
    prefixes
}

fn directories(root: &Path, relative: &str) -> impl Iterator<Item = PathBuf> {
    let pattern = format!("{}/{relative}", literal_save_pattern(root));
    glob::glob(&pattern)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|path| path.is_dir())
        .map(|path| canonical_path(&path))
}

fn existing_patterns(root: &Path, relative: &str) -> Vec<String> {
    let (parent, leaf) = split_static_parent(relative);
    let mut parents = vec![root.to_path_buf()];
    for name in parent.split('/').filter(|name| !name.is_empty()) {
        parents = parents
            .iter()
            .flat_map(|parent| {
                let exact = parent.join(name);
                if exact.is_dir() {
                    vec![exact]
                } else {
                    std::fs::read_dir(parent)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|entry| {
                            entry.file_name().eq_ignore_ascii_case(name) && entry.path().is_dir()
                        })
                        .map(|entry| entry.path())
                        .collect()
                }
            })
            .collect();
    }
    parents
        .into_iter()
        .map(|path| format!("{}/{leaf}", literal_save_pattern(&path)))
        .filter(|pattern| glob::glob(pattern).is_ok_and(|mut hits| hits.next().is_some()))
        .collect()
}

fn split_static_parent(relative: &str) -> (&str, &str) {
    let literal_end = crate::snapshot::first_wildcard(relative).unwrap_or(relative.len());
    let file_start = relative[..literal_end]
        .rfind('/')
        .map_or(0, |slash| slash + 1);
    relative.split_at(file_start)
}

fn launch_path<'a>(options: &'a str, variable: &str) -> Option<&'a str> {
    let (assignments, _) = options.split_once("%command%")?;
    let key = format!("{variable}=");
    for (index, _) in assignments.match_indices(&key) {
        if index > 0 && !assignments.as_bytes()[index - 1].is_ascii_whitespace() {
            continue;
        }
        let rest = &assignments[index + key.len()..];
        return match rest.as_bytes().first()? {
            b'\'' => rest[1..].split_once('\'').map(|(path, _)| path),
            b'"' => rest[1..].split_once('"').map(|(path, _)| path),
            _ => rest.split_ascii_whitespace().next(),
        };
    }
    None
}
