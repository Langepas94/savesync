use crate::paths::{canonical_path, Roots};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(super) fn game_prefixes(install: &Path, roots: &Roots) -> BTreeSet<PathBuf> {
    let mut configs: BTreeSet<_> = roots.xdg_config.iter().cloned().collect();
    if let Some(home) = &roots.home {
        configs.insert(home.join(".config"));
        configs.insert(home.join(".var/app/net.lutris.Lutris/config"));
        configs.insert(home.join(".var/app/com.heroicgameslauncher.hgl/config"));
    }
    let mut found = BTreeSet::new();
    for config in configs {
        found.extend(lutris_prefixes(&config, install));
        found.extend(heroic_prefixes(&config, install));
    }
    found
}

#[derive(Deserialize)]
struct LutrisConfig {
    game: LutrisGame,
}

#[derive(Deserialize)]
struct LutrisGame {
    exe: PathBuf,
    prefix: PathBuf,
}

fn lutris_prefixes(config: &Path, install: &Path) -> impl Iterator<Item = PathBuf> {
    let install = canonical_path(install);
    std::fs::read_dir(config.join("lutris/games"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(move |file| {
            let path = file.path();
            if !matches!(path.extension()?.to_str()?, "yml" | "yaml") {
                return None;
            }
            let bytes = std::fs::read(path).ok()?;
            let game = serde_yaml::from_slice::<LutrisConfig>(&bytes).ok()?.game;
            (game.exe.is_absolute()
                && game.prefix.is_absolute()
                && canonical_path(&game.exe).starts_with(&install))
            .then_some(game.prefix)
        })
}

#[derive(Deserialize)]
struct LegendaryInstall {
    install_path: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeroicGame {
    wine_prefix: PathBuf,
}

fn heroic_prefixes(config: &Path, install: &Path) -> BTreeSet<PathBuf> {
    let install = canonical_path(install);
    let heroic = config.join("heroic");
    let mut result = BTreeSet::new();
    for registry in [
        config.join("legendary/installed.json"),
        heroic.join("legendaryConfig/legendary/installed.json"),
    ] {
        let Ok(bytes) = std::fs::read(registry) else {
            continue;
        };
        let Ok(games) =
            serde_json::from_slice::<std::collections::BTreeMap<String, LegendaryInstall>>(&bytes)
        else {
            continue;
        };
        for (id, entry) in games {
            if !id
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
                || !entry.install_path.is_absolute()
                || canonical_path(&entry.install_path) != install
            {
                continue;
            }
            let Ok(bytes) = std::fs::read(heroic.join("GamesConfig").join(format!("{id}.json")))
            else {
                continue;
            };
            let Ok(settings) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let Some(game) = settings.get(&id) else {
                continue;
            };
            if let Ok(game) = HeroicGame::deserialize(game) {
                if game.wine_prefix.is_absolute() {
                    result.insert(game.wine_prefix);
                }
            }
        }
    }
    result
}
