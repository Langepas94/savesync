//! Платформы, корневые пути и резолвер плейсхолдеров манифестов.
//! Набор плейсхолдеров — docs/manifests-schema.md §5.

use std::path::PathBuf;

/// Целевая ОС для резолва путей манифеста.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    /// Linux натив или SteamOS; Proton-сейвы резолвятся через
    /// `<steamCompat>/<appid>/...` внутри Steam-библиотек.
    Linux,
}

impl Platform {
    /// Платформа, на которой собран бинарь.
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Набор корневых директорий, в которые резолвятся плейсхолдеры манифестов.
///
/// Заполняется per-platform: на Windows `xdg_*` и `steam_libraries` (для
/// Proton) обычно пусты, на Linux — `win_*`. Пустой корень означает
/// «плейсхолдер не резолвится».
#[derive(Debug, Clone, Default)]
pub struct Roots {
    pub home: Option<PathBuf>,
    /// `%APPDATA%` (roaming).
    pub win_appdata: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    pub win_local_appdata: Option<PathBuf>,
    /// `Saved Games` в профиле пользователя.
    pub win_saved_games: Option<PathBuf>,
    /// Документы пользователя (Windows: `Documents`, Linux: `~/Documents`).
    pub documents: Option<PathBuf>,
    /// `$XDG_DATA_HOME` или `~/.local/share`.
    pub xdg_data: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` или `~/.config`.
    pub xdg_config: Option<PathBuf>,
    /// Корни Steam-библиотек (для Proton-маппинга `<steamCompat>`):
    /// `<lib>/steamapps/compatdata/<appid>/...`.
    pub steam_libraries: Vec<PathBuf>,
    /// Game installation directory by Steam application ID.
    pub steam_installations: std::collections::BTreeMap<u32, PathBuf>,
}

impl Roots {
    /// Корни текущей машины, собранные из переменных окружения.
    /// В тестах не используется — там корни конструируются явно.
    pub fn detect(platform: Platform) -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from);
        match platform {
            Platform::Windows => {
                let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
                let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
                let saved_games = home.as_ref().map(|h| h.join("Saved Games"));
                let documents = home.as_ref().map(|h| h.join("Documents"));
                let steam_libraries =
                    crate::steam::discovery::detect_roots(platform, home.as_deref());
                Roots {
                    home,
                    win_appdata: appdata,
                    win_local_appdata: local,
                    win_saved_games: saved_games,
                    documents,
                    steam_libraries,
                    ..Roots::default()
                }
            }
            Platform::Linux => {
                let xdg_data = std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
                let xdg_config = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|h| h.join(".config")));
                let documents = home.as_ref().map(|h| h.join("Documents"));
                let steam = crate::steam::discovery::detect_roots(platform, home.as_deref());
                Roots {
                    home,
                    xdg_data,
                    xdg_config,
                    documents,
                    steam_libraries: steam,
                    ..Roots::default()
                }
            }
        }
    }
}

/// Резолвит один паттерн манифеста в glob-паттерны на диске.
///
/// Плейсхолдеры: `<home>`, `<winAppData>`, `<winLocalAppData>`,
/// `<winSavedGames>`, `<documents>`, `<xdgData>`, `<xdgConfig>`,
/// `<steamCompat>` (Proton: один кандидат на каждую Steam-библиотеку).
///
/// Возвращает пустой вектор, если плейсхолдер не имеет корня на этой
/// платформе (например, `<winAppData>` на чистом Linux) или неизвестен.
pub fn resolve_pattern_multi(pattern: &str, roots: &Roots) -> Vec<String> {
    let (placeholder, rest) = match pattern.strip_prefix('<').and_then(|p| p.split_once('>')) {
        Some((ph, rest)) => (ph, rest.trim_start_matches(['/', '\\'])),
        None => return vec![pattern.to_string()], // не плейсхолдер — как есть
    };

    if placeholder == "steamCompat" {
        return roots
            .steam_libraries
            .iter()
            .map(|lib| join(lib, &format!("steamapps/compatdata/{rest}")))
            .collect();
    }
    if placeholder == "steamUserdata" {
        return roots
            .steam_libraries
            .iter()
            .map(|lib| join(lib, &format!("userdata/{rest}")))
            .collect();
    }
    if placeholder == "steamInstall" {
        return rest
            .split_once('/')
            .and_then(|(id, tail)| {
                Some((
                    roots.steam_installations.get(&id.parse::<u32>().ok()?)?,
                    tail,
                ))
            })
            .map(|(base, tail)| format!("{}/{tail}", literal_save_pattern(base)))
            .into_iter()
            .collect();
    }
    if placeholder == "steamCloudDocs" {
        let Some((id, tail)) = rest.split_once('/') else {
            return Vec::new();
        };
        let Some(folder) = id
            .parse::<u32>()
            .ok()
            .and_then(|id| roots.steam_installations.get(&id))
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
        else {
            return Vec::new();
        };
        return roots
            .home
            .as_ref()
            .map(|home| {
                join(
                    home,
                    &format!(".SteamCloud/*/{}/{tail}", glob::Pattern::escape(folder)),
                )
            })
            .into_iter()
            .collect();
    }

    let root = match placeholder {
        "home" => roots.home.as_ref(),
        "winAppData" => roots.win_appdata.as_ref(),
        "winLocalAppData" => roots.win_local_appdata.as_ref(),
        "winSavedGames" => roots.win_saved_games.as_ref(),
        "documents" => roots.documents.as_ref(),
        "xdgData" => roots.xdg_data.as_ref(),
        "xdgConfig" => roots.xdg_config.as_ref(),
        _ => None, // неизвестный плейсхолдер — пропускаем, не падаем
    };

    root.map(|r| join(r, rest)).into_iter().collect()
}

/// Shared representation of a Windows save rule and its Wine/Proton equivalent.
pub(crate) fn windows_save_pattern(pattern: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let Some((_, tail)) = pattern
        .strip_prefix("<steamCompat>/")
        .and_then(|value| value.split_once('/'))
    else {
        return Cow::Borrowed(pattern);
    };
    let Some((_, relative)) = tail
        .strip_prefix("pfx/drive_c/users/")
        .and_then(|value| value.split_once('/'))
    else {
        return Cow::Borrowed(pattern);
    };
    [
        ("AppData/Roaming/", "<winAppData>/"),
        ("AppData/Local/", "<winLocalAppData>/"),
        ("AppData/LocalLow/", "<home>/AppData/LocalLow/"),
        ("Saved Games/", "<winSavedGames>/"),
        ("Documents/", "<documents>/"),
        ("My Documents/", "<documents>/"),
    ]
    .iter()
    .find_map(|(prefix, placeholder)| {
        relative
            .strip_prefix(prefix)
            .map(|rest| Cow::Owned(format!("{placeholder}{rest}")))
    })
    .unwrap_or(Cow::Borrowed(pattern))
}

/// Одиночный резолв для паттернов без мульти-корней (тесты, простые вызовы).
pub fn resolve_pattern(pattern: &str, roots: &Roots) -> Option<String> {
    resolve_pattern_multi(pattern, roots).into_iter().next()
}

/// Convert a literal filesystem path to a pattern usable by the save scanner.
pub fn literal_save_pattern(path: &std::path::Path) -> String {
    let path = canonical_path(path);
    let pattern = path.to_string_lossy();
    #[cfg(windows)]
    let pattern = pattern.replace('\\', "/");
    glob::Pattern::escape(&pattern)
}

/// Canonicalize trusted path roots without exposing Windows verbatim prefixes
/// to glob's '?' parser. Missing suffixes are preserved for first-save restore.
pub fn canonical_path(path: &std::path::Path) -> PathBuf {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    let canonical = loop {
        if let Ok(found) = ancestor.canonicalize() {
            break found;
        }
        let Some(name) = ancestor.file_name() else {
            return path.to_path_buf();
        };
        suffix.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return path.to_path_buf();
        };
        ancestor = parent;
    };
    let text = canonical.to_string_lossy();
    let mut canonical = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        canonical
    };
    for component in suffix.into_iter().rev() {
        canonical.push(component);
    }
    canonical
}

pub fn canonical_pattern(resolved: &str) -> String {
    let (base, rest) = crate::snapshot::split_location_pattern(resolved);
    let base = crate::snapshot::unescape_pattern(base);
    let mut canonical = literal_save_pattern(std::path::Path::new(&base));
    if !canonical.ends_with('/') && !rest.is_empty() {
        canonical.push('/');
    }
    canonical.push_str(rest);
    canonical
}

/// Trusted pattern roots may resolve through Steam's aliases; descendants may
/// not escape that root through a symlink while collecting actual save data.
pub fn validate_source_path(base: &std::path::Path, path: &std::path::Path) -> std::io::Result<()> {
    if !path.starts_with(base) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "save path is outside its location",
        ));
    }
    for ancestor in path.ancestors().take_while(|ancestor| *ancestor != base) {
        if std::fs::symlink_metadata(ancestor)?
            .file_type()
            .is_symlink()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "symlink inside save location",
            ));
        }
    }
    Ok(())
}

fn join(root: &std::path::Path, rest: &str) -> String {
    let mut s = glob::Pattern::escape(&root.to_string_lossy().replace('\\', "/"));
    if !rest.is_empty() {
        if !s.ends_with('/') {
            s.push('/');
        }
        s.push_str(&rest.replace('\\', "/"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_roots() -> Roots {
        Roots {
            home: Some(PathBuf::from(r"C:\Users\gamer")),
            win_appdata: Some(PathBuf::from(r"C:\Users\gamer\AppData\Roaming")),
            win_local_appdata: Some(PathBuf::from(r"C:\Users\gamer\AppData\Local")),
            win_saved_games: Some(PathBuf::from(r"C:\Users\gamer\Saved Games")),
            documents: Some(PathBuf::from(r"C:\Users\gamer\Documents")),
            ..Roots::default()
        }
    }

    fn linux_roots() -> Roots {
        Roots {
            home: Some(PathBuf::from("/home/deck")),
            xdg_data: Some(PathBuf::from("/home/deck/.local/share")),
            xdg_config: Some(PathBuf::from("/home/deck/.config")),
            documents: Some(PathBuf::from("/home/deck/Documents")),
            steam_libraries: vec![
                PathBuf::from("/home/deck/.local/share/Steam"),
                PathBuf::from("/run/media/mmcblk0p1"),
            ],
            ..Roots::default()
        }
    }

    #[test]
    fn windows_placeholders() {
        let r = windows_roots();
        assert_eq!(
            resolve_pattern("<winAppData>/SomeGame/saves/*.sav", &r).as_deref(),
            Some("C:/Users/gamer/AppData/Roaming/SomeGame/saves/*.sav")
        );
        assert_eq!(
            resolve_pattern("<winSavedGames>/Studio/Game", &r).as_deref(),
            Some("C:/Users/gamer/Saved Games/Studio/Game")
        );
        assert_eq!(
            resolve_pattern("<home>/Documents/My Games/Game", &r).as_deref(),
            Some("C:/Users/gamer/Documents/My Games/Game")
        );
    }

    #[test]
    fn linux_placeholders() {
        let r = linux_roots();
        assert_eq!(
            resolve_pattern("<xdgData>/game/saves", &r).as_deref(),
            Some("/home/deck/.local/share/game/saves")
        );
        assert_eq!(
            resolve_pattern("<home>/.local/share/Steam", &r).as_deref(),
            Some("/home/deck/.local/share/Steam")
        );
    }

    #[test]
    fn steam_compat_expands_per_library() {
        let r = linux_roots();
        let resolved = resolve_pattern_multi(
            "<steamCompat>/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing",
            &r,
        );
        assert_eq!(
            resolved,
            vec![
                "/home/deck/.local/share/Steam/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing",
                "/run/media/mmcblk0p1/steamapps/compatdata/1245620/pfx/drive_c/users/steamuser/AppData/Roaming/EldenRing",
            ]
        );
        // На Windows без Steam-библиотек Proton-маппинга нет.
        assert!(resolve_pattern_multi("<steamCompat>/1/pfx", &windows_roots()).is_empty());
    }

    #[test]
    fn cross_platform_mismatch_returns_none() {
        assert_eq!(resolve_pattern("<winAppData>/Game", &linux_roots()), None);
        assert_eq!(resolve_pattern("<xdgData>/game", &windows_roots()), None);
    }

    #[test]
    fn unknown_placeholder_is_skipped() {
        let r = windows_roots();
        assert!(resolve_pattern_multi("<someFuture>/x", &r).is_empty());
    }

    #[test]
    fn plain_path_passes_through() {
        let r = windows_roots();
        assert_eq!(
            resolve_pattern("D:/Games/saves", &r).as_deref(),
            Some("D:/Games/saves")
        );
    }
}
