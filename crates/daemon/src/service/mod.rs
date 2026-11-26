//! Ядро демона без транспорта: конфиг + манифесты + сканер + engine.
//! Покрыто тестами напрямую; IPC-сервер — тонкая обёртка над этим.

use std::io;
use std::path::PathBuf;

use savesync_core::backends::lan::LanServer;
use savesync_core::catalog::manifest::ManifestDb;
use savesync_core::catalog::scanner::FoundGame;
use savesync_core::config::Config;
use savesync_core::platform::paths::{Platform, Roots};

pub struct Runtime {
    pub shutdown_requested: bool,
    pub config: Config,
    pub(crate) base_manifests: ManifestDb,
    pub manifests: ManifestDb,
    pub platform: Platform,
    pub roots: Roots,
    /// Локальное состояние движка (индекс, tmp, бэкапы).
    pub state_dir: PathBuf,
    pub last_scan: Vec<FoundGame>,
    pub hidden_games: std::collections::BTreeMap<String, String>,
    pub lan_server: Option<LanServer>,
    pub serving: bool,
    pub oauth_jobs:
        std::collections::HashMap<savesync_core::auth::oauth::Provider, authorization::OAuthJob>,
}

impl Runtime {
    /// Собирает runtime из конфига. Манифесты загружаются из
    /// `config.manifests_dir` (может быть пустой базой) плюс custom-правила
    /// пользователя из `<state_dir>/custom/*.yaml` — они никогда не
    /// перезаписываются обновлением базы.
    pub fn new(config: Config, state_dir: PathBuf) -> io::Result<Self> {
        savesync_core::sync::engine::recover_state(&state_dir)?;
        let mut manifests = savesync_core::catalog::bundled_manifests::load();
        let installed_database = state_dir
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("manifests");
        for dir in [
            Some(&installed_database).filter(|dir| dir.is_dir()),
            config.manifests_dir.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let (db, errors) = savesync_core::catalog::manifest::load_manifest_tree(dir);
            for e in &errors {
                eprintln!("manifest load warning: {e}");
            }
            for manifest in db.iter() {
                manifests.insert(manifest.clone());
            }
        }
        let platform = Platform::current();
        let roots = Roots::detect(platform);
        let hidden_games = match std::fs::read(state_dir.join("hidden-games.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Default::default(),
            Err(e) => return Err(e),
        };
        let mut runtime = Runtime {
            shutdown_requested: false,
            config,
            base_manifests: manifests,
            manifests: ManifestDb::default(),
            platform,
            roots,
            state_dir,
            last_scan: Vec::new(),
            hidden_games,
            lan_server: None,
            serving: false,
            oauth_jobs: Default::default(),
        };
        runtime.refresh_catalog();
        Ok(runtime)
    }
}
mod backups;
mod catalog;
pub(crate) use catalog::AddGameError;
mod channels;
mod configuration;
mod history;
mod scheduled_backups;
mod sync;
#[cfg(test)]
#[path = "../tests/service.rs"]
mod tests;

mod authorization;
pub(crate) use authorization::cloud_check;

mod networking;
pub(crate) use networking::lan_presence;
