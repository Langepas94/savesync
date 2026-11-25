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
use crate::manifest::{Os, PathRule};

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
            if root.is_empty() || path.is_empty() {
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
            // TODO(generator): rootoverrides (per-OS замена root/path) —
            // редкий случай, сейчас берём базовый root и логируем пропуск.
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
        "WinSavedGames" => ("<winSavedGames>", Os::Windows),
        "WinMyDocuments" | "SteamCloudDocuments" => ("<documents>", Os::Windows),
        "LinuxXdgDataHome" => ("<xdgData>", Os::Linux),
        "LinuxXdgConfigHome" => ("<xdgConfig>", Os::Linux),
        "LinuxHome" | "MacHome" => ("<home>", Os::Any),
        // GameInstall — привязка к библиотеке (плейсхолдера нет, TODO),
        // WinAppDataLocalLow — нет плейсхолдера, MacAppSupport — macOS.
        _ => return None,
    })
}

/// Куда Win-root ложится внутри Proton-префикса
/// (`pfx/drive_c/users/steamuser/<это>`).
fn proton_dir(win_root: &str) -> Option<&'static str> {
    Some(match win_root {
        "<winAppData>" => "AppData/Roaming",
        "<winLocalAppData>" => "AppData/Local",
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
        let glob = if sf.recursive {
            format!("**/{pat}")
        } else {
            pat.clone()
        };
        let full = join_pattern(&format!("{placeholder}/{path}"), &glob);
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
mod tests {
    use super::*;
    use crate::steam::kv_binary::Value;

    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }

    /// Синтетическая запись appinfo с UFS как у реальной игры.
    fn entry(appid: u32, name: &str, savefiles: Value) -> AppInfoEntry {
        let common = map(vec![("name", s(name)), ("type", s("Game"))]);
        let ufs = map(vec![("savefiles", savefiles)]);
        AppInfoEntry {
            appid,
            change_number: 0,
            last_updated: 0,
            tree: vec![(
                "appinfo".into(),
                map(vec![("common", common), ("ufs", ufs)]),
            )],
        }
    }

    #[test]
    fn extracts_savefiles_new_and_old_pattern_format() {
        let sf0 = map(vec![
            ("root", s("WinAppDataRoaming")),
            ("path", s("EldenRing")),
            ("patterns", map(vec![("0", s("*.sl2"))])),
            ("recursive", s("1")),
        ]);
        let sf1 = map(vec![
            ("root", s("LinuxXdgDataHome")),
            ("path", s("game\\saves")),
            ("pattern", s("*.sav")),
        ]);
        let savefiles = map(vec![("0", sf0), ("1", sf1)]);
        let e = entry(1, "Game", savefiles);

        let files = e.ufs_savefiles();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].patterns, vec!["*.sl2"]);
        assert!(files[0].recursive);
        assert_eq!(files[1].path, "game/saves"); // backslashes нормализованы
        assert_eq!(files[1].patterns, vec!["*.sav"]);
    }

    #[test]
    fn no_ufs_means_empty() {
        let common = map(vec![("name", s("Tool")), ("type", s("Tool"))]);
        let e = AppInfoEntry {
            appid: 1,
            change_number: 0,
            last_updated: 0,
            tree: vec![("appinfo".into(), map(vec![("common", common)]))],
        };
        assert_eq!(e.app_type(), Some("Tool"));
        assert!(e.ufs_savefiles().is_empty());
    }

    #[test]
    fn windows_root_generates_proton_rule_too() {
        let sf = UfsSaveFile {
            root: "WinAppDataRoaming".into(),
            path: "EldenRing".into(),
            patterns: vec!["*.sl2".into()],
            recursive: false,
        };
        let rules = to_path_rules(1245620, &sf);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].pattern, "<winAppData>/EldenRing/*.sl2");
        assert_eq!(rules[0].os, Os::Windows);
        assert_eq!(
            rules[1].pattern,
            "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing/*.sl2"
        );
        assert_eq!(rules[1].os, Os::Linux);
    }

    #[test]
    fn steamid_placeholders_become_globs() {
        let sf = UfsSaveFile {
            root: "WinAppDataRoaming".into(),
            path: "Game/{64BitSteamID}".into(),
            patterns: vec!["*".into()],
            recursive: false,
        };
        let rules = to_path_rules(1, &sf);
        assert!(rules[0].pattern.contains("/*/*"));
    }

    #[test]
    fn unsupported_roots_are_skipped() {
        let sf = UfsSaveFile {
            root: "GameInstall".into(),
            path: "saves".into(),
            patterns: vec![],
            recursive: false,
        };
        assert!(to_path_rules(1, &sf).is_empty());
        let sf2 = UfsSaveFile {
            root: "MacAppSupport".into(),
            path: "saves".into(),
            patterns: vec![],
            recursive: false,
        };
        assert!(to_path_rules(1, &sf2).is_empty());
    }

    #[test]
    fn linux_roots_map_to_xdg() {
        let sf = UfsSaveFile {
            root: "LinuxXdgDataHome".into(),
            path: "game".into(),
            patterns: vec!["*.sav".into()],
            recursive: true,
        };
        let rules = to_path_rules(1, &sf);
        assert_eq!(rules.len(), 1); // без Proton-двойника
        assert_eq!(rules[0].pattern, "<xdgData>/game/**/*.sav");
        assert_eq!(rules[0].os, Os::Linux);
    }
}
