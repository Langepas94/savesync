//! Steam-метаданные: локальный appinfo (`appinfo.vdf`), библиотеки
//! (`libraryfolders.vdf`), установленные игры (`appmanifest_*.acf`),
//! Proton-префиксы (`compatdata/<appid>`) и матчинг с `savesync-manifests`.
//!
//! Полностью наш источник данных — чужих API в рантайме нет (HANDOFF §1).

pub mod appinfo;
pub mod discovery;
pub mod kv_binary;
pub mod ufs;
pub mod vdf_text;

use std::path::{Path, PathBuf};

use crate::game_key;
use crate::manifest::ManifestDb;

/// Установленная Steam-игра (из appmanifest + compatdata).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamApp {
    pub appid: u32,
    pub name: String,
    /// `installdir` из appmanifest (относительно `steamapps/common`).
    pub installdir: String,
    /// Корень библиотеки, где установлена игра.
    pub library: PathBuf,
    /// Proton-префикс, если есть: `<library>/steamapps/compatdata/<appid>`.
    pub compat_prefix: Option<PathBuf>,
}

/// Steam-библиотека (корень с `steamapps/`).
#[derive(Debug, Clone)]
pub struct Library {
    pub root: PathBuf,
    pub apps: Vec<SteamApp>,
}

/// Парсит `libraryfolders.vdf` -> список корней библиотек.
///
/// Современный формат: `"libraryfolders" { "0" { "path" "..." } ... }`.
/// Устаревший (плоские пары `"0" "C:\\Steam"`) поддерживается тоже.
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    let Ok(doc) = vdf_text::parse(text) else {
        return Vec::new();
    };
    let root = vdf_text::Node::Map(doc);
    let Some(folders) = root.get(&["libraryfolders"]).and_then(|n| n.as_map()) else {
        return Vec::new();
    };

    let mut roots = Vec::new();
    for (key, node) in folders {
        if !key.chars().all(|c| c.is_ascii_digit()) {
            continue; // пропускаем служебные поля ("contentstatsid" и т.п.)
        }
        match node {
            vdf_text::Node::Map(m) => {
                if let Some(path) = m
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("path"))
                    .and_then(|(_, v)| v.as_str())
                {
                    roots.push(PathBuf::from(path));
                }
            }
            vdf_text::Node::Str(path) => roots.push(PathBuf::from(path)),
        }
    }
    roots
}

/// Парсит `appmanifest_<appid>.acf` -> (appid, name, installdir).
pub fn parse_appmanifest(text: &str) -> Option<(u32, String, String)> {
    let doc = vdf_text::parse(text).ok()?;
    let root = vdf_text::Node::Map(doc);
    let appid: u32 = root.get_str(&["AppState", "appid"])?.parse().ok()?;
    let name = root.get_str(&["AppState", "name"])?.to_string();
    let installdir = root.get_str(&["AppState", "installdir"])?.to_string();
    Some((appid, name, installdir))
}

/// Сканирует библиотеку: все `steamapps/appmanifest_*.acf` + compatdata.
/// Битые манифесты пропускаются — одна кривая запись Steam не должна
/// ронять сканирование.
pub fn scan_library(root: &Path) -> Library {
    let steamapps = root.join("steamapps");
    let mut apps = Vec::new();

    if let Ok(rd) = std::fs::read_dir(&steamapps) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Some((appid, app_name, installdir)) = parse_appmanifest(&text) else {
                continue;
            };
            let compat = steamapps.join("compatdata").join(appid.to_string());
            apps.push(SteamApp {
                appid,
                name: app_name,
                installdir,
                library: root.to_path_buf(),
                compat_prefix: compat.is_dir().then_some(compat),
            });
        }
    }

    apps.sort_by_key(|a| a.appid);
    Library {
        root: root.to_path_buf(),
        apps,
    }
}

/// Находит все библиотеки Steam от корня установки (`<steam>/steamapps/
/// libraryfolders.vdf`) и сканирует каждую. Сам корень — тоже библиотека.
pub fn discover_libraries(steam_root: &Path) -> Vec<Library> {
    let mut roots = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&vdf) {
        for path in parse_library_folders(&text) {
            if !roots.contains(&path) {
                roots.push(path);
            }
        }
    }
    roots
        .iter()
        .filter(|r| r.join("steamapps").is_dir())
        .map(|r| scan_library(r))
        .collect()
}

/// Как установленная Steam-игра сматчилась на манифест.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// По `ids.steam` в манифесте — надёжный матч.
    SteamId,
    /// По slug имени (fallback; требует подтверждения при неоднозначности).
    NameSlug,
}

/// Результат матчинга Steam-игры на запись `savesync-manifests`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamMatch {
    pub appid: u32,
    pub steam_name: String,
    pub game_key: String,
    pub matched_by: MatchKind,
}

/// Матчит установленные Steam-игры на базу манифестов:
/// сначала по `ids.steam`, затем по slug имени.
pub fn match_manifests(db: &ManifestDb, apps: &[SteamApp]) -> Vec<SteamMatch> {
    let mut out = Vec::new();
    for app in apps {
        let appid = app.appid.to_string();
        let by_id = db
            .iter()
            .find(|m| !m.slug.starts_with("custom:") && m.ids.steam.iter().any(|id| id == &appid));
        let (m, kind) = match by_id {
            Some(m) => (m, MatchKind::SteamId),
            None => {
                let slug = game_key::slugify(&app.name);
                match db.get(&slug) {
                    Some(m) if m.ids.steam.is_empty() || m.ids.steam.contains(&appid) => {
                        (m, MatchKind::NameSlug)
                    }
                    Some(_) => continue,
                    None => continue,
                }
            }
        };
        out.push(SteamMatch {
            appid: app.appid,
            steam_name: app.name.clone(),
            game_key: m.slug.clone(),
            matched_by: kind,
        });
    }
    out
}

#[cfg(test)]
#[path = "../../tests/platform/steam.rs"]
mod tests;
