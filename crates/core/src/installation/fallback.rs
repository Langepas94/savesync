use crate::manifest::{Os, PathRule};
use crate::paths::{literal_save_pattern, Platform};
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
                let folder_name = crate::game_key::slugify(&entry.file_name().to_string_lossy());
                let is_save_dir = matches!(
                    folder_name.replace('-', "").as_str(),
                    "save" | "saves" | "savegames" | "savegame" | "savedata" | "gamesaves"
                );
                pending.push((path, save_tree || is_save_dir, depth + 1));
            } else if kind.is_file() && is_save_file(&path, save_tree || depth == 0) {
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
