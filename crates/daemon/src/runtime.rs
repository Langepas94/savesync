//! Ядро демона без транспорта: конфиг + манифесты + сканер + engine.
//! Покрыто тестами напрямую; IPC-сервер — тонкая обёртка над этим.

use std::io;
use std::path::PathBuf;

use savesync_core::backend::{Backend, FsBackend};
use savesync_core::config::Config;
use savesync_core::engine::{Engine, PullOutcome, PushOutcome};
use savesync_core::gdrive::GoogleDrive;
use savesync_core::ipc::{
    GameStatus, ResponsePayload, ScanPayload, ScannedGame, StatusPayload, SyncPayload, SyncResult,
};
use savesync_core::lan::LanBackend;
use savesync_core::manifest::{Manifest, ManifestDb};
use savesync_core::paths::{Platform, Roots};
use savesync_core::scanner::{self, FoundGame};
use savesync_core::yandex::YandexDisk;

pub struct Runtime {
    pub config: Config,
    pub manifests: ManifestDb,
    pub platform: Platform,
    pub roots: Roots,
    /// Локальное состояние движка (индекс, tmp, бэкапы).
    pub state_dir: PathBuf,
    pub last_scan: Vec<FoundGame>,
    pub lan_server: Option<savesync_core::lan::LanServer>,
    pub serving: bool,
}

impl Runtime {
    /// Собирает runtime из конфига. Манифесты загружаются из
    /// `config.manifests_dir` (может быть пустой базой) плюс custom-правила
    /// пользователя из `<state_dir>/custom/*.yaml` — они никогда не
    /// перезаписываются обновлением базы.
    pub fn new(config: Config, state_dir: PathBuf) -> io::Result<Self> {
        let mut manifests = match &config.manifests_dir {
            Some(dir) => {
                let (db, errors) = savesync_core::manifest::load_manifest_tree(dir);
                for e in &errors {
                    eprintln!("manifest load warning: {e}");
                }
                db
            }
            None => ManifestDb::default(),
        };
        let custom_dir = state_dir.join("custom");
        if custom_dir.is_dir() {
            let (custom_db, errors) = savesync_core::manifest::load_manifest_tree(&custom_dir);
            for e in &errors {
                eprintln!("custom manifest load warning: {e}");
            }
            for m in custom_db.iter() {
                manifests.insert(m.clone());
            }
        }
        let platform = Platform::current();
        let roots = Roots::detect(platform);
        let last_scan = scanner::scan(&manifests, platform, &roots);
        Ok(Runtime {
            config,
            manifests,
            platform,
            roots,
            state_dir,
            last_scan,
            lan_server: None,
            serving: false,
        })
    }
}
mod backups;
mod catalog;
mod channels;
mod configuration;
mod history;
mod scheduled_backups;
mod sync;
#[cfg(test)]
mod tests;
