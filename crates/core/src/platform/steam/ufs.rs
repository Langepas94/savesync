//! Извлечение UFS (User File System) — конфигов Steam Cloud из записей
//! `appinfo.vdf`. Это полностью наш источник путей сейвов для генератора
//! манифестов (HANDOFF §1: локальный appinfo → репо savesync-manifests).
//!
//! Дерево: `appinfo/ufs/savefiles/<N>` = { root, path, pattern|patterns,
//! recursive, rootoverrides }. Root — символический (WinAppDataRoaming,
//! LinuxXdgDataHome, ...). Пути могут содержать {64BitSteamID} и
//! {Steam3AccountID} — заменяем на glob `*`.
//!
//! Proton-маппинг: для Win*-корней дополнительно генерируем linux-правило
//! через `<steamCompat>/<appid>/pfx/drive_c/users/steamuser/...` — так
//! выглядит тот же сейв на Steam Deck, запускающем Windows-сборку.

use super::appinfo::AppInfoEntry;
use super::kv_binary::Value;
use crate::catalog::manifest::{Os, PathRule};

/// Shared generator for runtime discovery and the standalone manifest tool.
pub fn manifest_from_entry(entry: &AppInfoEntry) -> Option<crate::catalog::manifest::Manifest> {
    use crate::catalog::manifest::{Ids, Manifest};
    if !entry.app_type()?.eq_ignore_ascii_case("game") {
        return None;
    }
    let name = entry.display_name()?;
    let saves = entry.ufs_savefiles();
    let mut paths: Vec<_> = saves
        .iter()
        .flat_map(|save| to_path_rules(entry.appid, save))
        .collect();
    let override_nodes = get(&entry.tree, &["appinfo", "ufs", "rootoverrides"]);
    if let Some(overrides) = override_nodes.and_then(Value::as_map) {
        for (_, value) in overrides {
            let Some(fields) = value.as_map() else {
                continue;
            };
            if !map_str(fields, "os").is_some_and(|os| os.eq_ignore_ascii_case("linux"))
                || map_str(fields, "oscompare").is_some_and(|compare| compare != "=")
            {
                continue;
            }
            let (Some(original), Some(replacement)) =
                (map_str(fields, "root"), map_str(fields, "useinstead"))
            else {
                continue;
            };
            let added = normalize_slashes(&map_str(fields, "addpath").unwrap_or_default());
            let replace_path = map_str(fields, "replacepath").is_some_and(|value| value == "1");
            for save in saves
                .iter()
                .filter(|save| save.root.eq_ignore_ascii_case(&original))
            {
                for file in &save.patterns {
                    let native_path = if replace_path {
                        added.clone()
                    } else {
                        join_pattern(&added, &save.path)
                    };
                    let mut override_rule = save.clone();
                    override_rule.root.clone_from(&replacement);
                    override_rule.path = native_path;
                    override_rule.patterns = vec![file.clone()];
                    let mut original_rule = save.clone();
                    original_rule.patterns = vec![file.clone()];
                    let Some(source) = to_path_rules(entry.appid, &original_rule)
                        .into_iter()
                        .find(|rule| rule.os != Os::Linux)
                    else {
                        continue;
                    };
                    let location = format!(
                        "windows/{}",
                        crate::platform::paths::windows_save_pattern(&source.pattern)
                    );
                    paths.extend(
                        to_path_rules(entry.appid, &override_rule)
                            .into_iter()
                            .filter(|rule| rule.os != Os::Windows)
                            .map(|rule| PathRule {
                                location: Some(location.clone()),
                                ..rule
                            }),
                    );
                }
            }
        }
    }
    paths.sort_by(|a, b| a.pattern.cmp(&b.pattern));
    paths.dedup_by(|a, b| a.pattern == b.pattern && a.os == b.os);
    let manifest = Manifest {
        schema: 1,
        name: name.into(),
        slug: crate::catalog::game_key::slugify(name),
        catalog_key: None,
        installed_at: None,
        aliases: vec![name.into()],
        ids: Ids {
            steam: vec![entry.appid.to_string()],
            ..Ids::default()
        },
        paths,
        registry: vec![],
        portability: Default::default(),
    };
    manifest.validate().is_empty().then_some(manifest)
}

/// Одна запись `ufs/savefiles/N`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UfsSaveFile {
    pub root: String,
    pub path: String,
    pub patterns: Vec<String>,
    pub recursive: bool,
}

impl AppInfoEntry {
    /// Имя приложения (`appinfo/common/name`).
    pub fn display_name(&self) -> Option<&str> {
        self.name()
    }

    /// Тип приложения (`appinfo/common/type`: Game, Tool, DLC, ...).
    pub fn app_type(&self) -> Option<&str> {
        get_str(&self.tree, &["appinfo", "common", "type"])
    }

    /// Записи UFS savefiles. Не-игры и игры без UFS дают пустой список.
    pub fn ufs_savefiles(&self) -> Vec<UfsSaveFile> {
        let mut out = Vec::new();
        let Some(Value::Map(savefiles)) = get(&self.tree, &["appinfo", "ufs", "savefiles"]) else {
            return out;
        };
        for (_, node) in savefiles {
            let Some(map) = node.as_map() else { continue };
            let root = map_str(map, "root").unwrap_or_default();
            let path = map_str(map, "path").unwrap_or_default();
            if root.is_empty() {
                continue;
            }
            // patterns: новый формат — нумерованный map, старый — одна строка.
            let mut patterns = Vec::new();
            if let Some(Value::Map(ps)) = map_get(map, "patterns") {
                for (_, p) in ps {
                    if let Some(s) = p.as_str() {
                        patterns.push(s.to_string());
                    }
                }
            } else if let Some(p) = map_str(map, "pattern") {
                patterns.push(p);
            }
            if patterns.is_empty() {
                patterns.push("*".into());
            }
            let recursive = map_str(map, "recursive").map(|v| v == "1").unwrap_or(false);
            // Platform-specific root overrides are applied by manifest_from_entry.
            out.push(UfsSaveFile {
                root,
                path: normalize_slashes(&path),
                patterns,
                recursive,
            });
        }
        out
    }
}

/// Маппинг символического UFS-root на наш плейсхолдер и ОС.
/// Возвращает (placeholder, os) или None (не поддерживаем/не переносимо).
fn map_root(root: &str) -> Option<(&'static str, Os)> {
    Some(match root {
        "WinAppDataRoaming" => ("<winAppData>", Os::Windows),
        "WinAppDataLocal" => ("<winLocalAppData>", Os::Windows),
        "WinAppDataLocalLow" => ("<home>/AppData/LocalLow", Os::Windows),
        "WinSavedGames" => ("<winSavedGames>", Os::Windows),
        "WinMyDocuments" => ("<documents>", Os::Windows),
        "SteamCloudDocuments" => ("<steamCloudDocs>", Os::Linux),
        "GameInstall" => ("<base>", Os::Any),
        "LinuxXdgDataHome" => ("<xdgData>", Os::Linux),
        "LinuxXdgConfigHome" => ("<xdgConfig>", Os::Linux),
        "LinuxHome" => ("<home>", Os::Linux),
        _ => return None,
    })
}

/// Куда Win-root ложится внутри Proton-префикса
/// (`pfx/drive_c/users/steamuser/<это>`).
fn proton_dir(win_root: &str) -> Option<&'static str> {
    Some(match win_root {
        "<winAppData>" => "AppData/Roaming",
        "<winLocalAppData>" => "AppData/Local",
        "<home>/AppData/LocalLow" => "AppData/LocalLow",
        "<winSavedGames>" => "Saved Games",
        "<documents>" => "Documents",
        _ => return None,
    })
}

/// Генерирует правила манифеста из UFS-записи.
///
/// Для Win-корней дополнительно эмитит Proton-правило (os: linux через
/// `<steamCompat>`): игра под Proton пишет в тот же логический путь
/// внутри префикса.
pub fn to_path_rules(appid: u32, sf: &UfsSaveFile) -> Vec<PathRule> {
    let mut rules = Vec::new();
    let Some((placeholder, os)) = map_root(&sf.root) else {
        return rules;
    };

    for pat in &sf.patterns {
        let path = subst_steamid(&sf.path);
        let path = path.trim_matches('/');
        let path = if path == "." { "" } else { path };
        if path
            .split('/')
            .chain(pat.split('/'))
            .any(|part| part == "..")
        {
            continue;
        }
        let glob = if sf.recursive {
            format!("**/{pat}")
        } else {
            pat.clone()
        };
        let full = if placeholder == "<steamCloudDocs>" {
            join_pattern(
                &join_pattern(&format!("{placeholder}/{appid}"), path),
                &glob,
            )
        } else {
            join_pattern(&join_pattern(placeholder, path), &glob)
        };
        rules.push(PathRule {
            location: None,
            pattern: full,
            os,
            tags: vec!["save".into()],
        });

        if let Some(dir) = proton_dir(placeholder) {
            let proton = format!("<steamCompat>/{appid}/pfx/drive_c/users/steamuser/{dir}");
            rules.push(PathRule {
                location: None,
                pattern: join_pattern(&format!("{proton}/{path}"), &glob),
                os: Os::Linux,
                tags: vec!["save".into()],
            });
        }
    }
    rules
}

/// `{64BitSteamID}` / `{Steam3AccountID}` в UFS-путях → glob `*`.
fn subst_steamid(path: &str) -> String {
    path.replace("{64BitSteamID}", "*")
        .replace("{Steam3AccountID}", "*")
        .replace("{64BitSteamId}", "*")
}

fn normalize_slashes(p: &str) -> String {
    p.replace('\\', "/")
}

fn join_pattern(base: &str, glob: &str) -> String {
    if glob.is_empty() {
        return base.to_string();
    }
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        glob.trim_start_matches('/')
    )
}

fn get<'a>(tree: &'a [(String, Value)], path: &[&str]) -> Option<&'a Value> {
    super::appinfo::get_in(tree, path)
}

fn get_str<'a>(tree: &'a [(String, Value)], path: &[&str]) -> Option<&'a str> {
    get(tree, path)?.as_str()
}

fn map_get<'a>(map: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn map_str(map: &[(String, Value)], key: &str) -> Option<String> {
    match map_get(map, key)? {
        Value::Str(s) => Some(s.clone()),
        Value::I32(i) => Some(i.to_string()),
        Value::U64(u) => Some(u.to_string()),
        Value::I64(i) => Some(i.to_string()),
        Value::Map(_) => None,
    }
}

#[cfg(test)]
#[path = "../../tests/platform/ufs.rs"]
mod tests;
