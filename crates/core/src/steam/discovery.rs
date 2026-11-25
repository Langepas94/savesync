//! Read-only discovery of Steam libraries and installed games from local files.
use super::{appinfo, discover_libraries, SteamApp};
use crate::{manifest::ManifestDb, paths::Platform};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn detect_roots(platform: Platform, home: Option<&Path>) -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os("SAVESYNC_STEAM_ROOT") {
        return vec![path.into()];
    }
    let mut roots = Vec::new();
    match platform {
        Platform::Linux => {
            if let Some(home) = home {
                roots.extend([
                    home.join(".local/share/Steam"),
                    home.join(".steam/steam"),
                    home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
                ]);
            }
        }
        Platform::Windows => {
            #[cfg(windows)]
            {
                use winreg::{
                    enums::{HKEY_CURRENT_USER, KEY_READ},
                    RegKey,
                };
                if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
                    .open_subkey_with_flags("Software\\Valve\\Steam", KEY_READ)
                {
                    if let Ok(path) = key.get_value::<String, _>("SteamPath") {
                        roots.push(path.into());
                    }
                }
            }
            for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
                if let Some(base) = std::env::var_os(variable) {
                    roots.push(PathBuf::from(base).join("Steam"));
                }
            }
        }
    }
    roots
}

/// Local UFS rules take precedence; canonical names and portability restrictions
/// remain those of the curated database. Only installed appids are imported.
pub fn discover(
    db: &mut ManifestDb,
    roots: &[PathBuf],
) -> (Vec<SteamApp>, Vec<PathBuf>, Vec<String>) {
    let mut apps = Vec::new();
    let mut libraries = Vec::new();
    let mut diagnostics = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        for library in discover_libraries(root) {
            let identity = crate::paths::canonical_path(&library.root);
            if seen.insert(identity.clone()) {
                apps.extend(library.apps);
                libraries.push(identity);
            }
        }
    }
    apps.sort_by_key(|app| app.appid);
    apps.dedup_by_key(|app| app.appid);
    let installed: HashSet<u32> = apps.iter().map(|app| app.appid).collect();
    for root in roots {
        let path = root.join("appcache/appinfo.vdf");
        if !path.is_file() {
            continue;
        }
        match appinfo::load(&path) {
            Ok(entries) => {
                for entry in entries
                    .iter()
                    .filter(|entry| installed.contains(&entry.appid))
                {
                    let Some(mut generated) = super::ufs::manifest_from_entry(entry) else {
                        continue;
                    };
                    let existing = db
                        .iter()
                        .find(|manifest| manifest.ids.steam.contains(&entry.appid.to_string()))
                        .cloned();
                    if let Some(mut curated) = existing {
                        let mut paths = generated.paths;
                        for rule in curated.paths {
                            if let Some(local) = paths
                                .iter_mut()
                                .find(|path| path.pattern == rule.pattern && path.os == rule.os)
                            {
                                if local.location.is_none() {
                                    local.location = rule.location;
                                }
                            } else {
                                paths.push(rule);
                            }
                        }
                        curated.paths = paths;
                        generated = curated;
                    } else if let Some(collision) = db.get(&generated.slug) {
                        if !collision.ids.steam.is_empty()
                            && !collision.ids.steam.contains(&entry.appid.to_string())
                        {
                            diagnostics.push(format!(
                                "ambiguous Steam game name: {} ({})",
                                generated.name, entry.appid
                            ));
                            continue;
                        }
                        generated.portability = collision.portability.clone();
                    }
                    db.insert(generated);
                }
            }
            Err(error) => diagnostics.push(format!("{}: {error}", path.display())),
        }
    }
    (apps, libraries, diagnostics)
}
