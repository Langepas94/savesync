use super::*;
use savesync_core::catalog::manifest::parse_manifest_str;
use savesync_core::config::BackendsConfig;
use savesync_core::ipc::{SyncPayload, SyncResult};

struct World {
    dir: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "savesync-test-daemon-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        World { dir }
    }

    /// Симулирует «устройство»: своя home-папка с сейвом и state_dir.
    fn device(&self, name: &str) -> Runtime {
        let home = self.dir.join(name).join("home");
        std::fs::create_dir_all(home.join("game/saves")).unwrap();
        std::fs::write(home.join("game/saves/slot1.sav"), format!("save of {name}")).unwrap();

        let manifest = parse_manifest_str(
            "schema: 1\nname: Test Game\nslug: test-game\npaths:\n  - pattern: \"<home>/game/saves\"\n",
        )
        .unwrap();
        Runtime {
            shutdown_requested: false,
            config: Config {
                version: 1,
                device_id: name.into(),
                device_name: name.into(),
                manifests_dir: None,
                backends: BackendsConfig {
                    fs_path: Some(self.dir.join("remote")),
                    ..BackendsConfig::default()
                },
                backups: Default::default(),
            },
            base_manifests: ManifestDb::from_entries(vec![manifest]),
            manifests: ManifestDb::default(),
            platform: Platform::Windows,
            roots: Roots {
                home: Some(home),
                ..Roots::default()
            },
            state_dir: self.dir.join(name).join("state"),
            last_scan: Vec::new(),
            hidden_games: Default::default(),
            lan_server: None,
            serving: false,
            oauth_jobs: Default::default(),
        }
    }

    fn save_of(&self, device: &str) -> String {
        std::fs::read_to_string(self.dir.join(device).join("home/game/saves/slot1.sav")).unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn sync_results(payload: SyncPayload) -> Vec<SyncResult> {
    payload.results
}

#[path = "service_backups.rs"]
mod backups;
#[path = "service_catalog.rs"]
mod catalog;
#[path = "service_history.rs"]
mod history;
#[path = "service_networking.rs"]
mod networking;
#[path = "service_sync.rs"]
mod sync;
