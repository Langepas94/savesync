use super::Runtime;
use savesync_core::backends::lan::LanServer;
use savesync_core::config::Config;
use std::io;
impl Runtime {
    pub fn prepare_shutdown(&mut self) {
        self.shutdown_requested = true;
        self.oauth_jobs.clear();
        self.lan_server = None;
        self.serving = false;
    }
    pub fn start_lan(&mut self) -> io::Result<()> {
        self.serving = true;
        if let Some(lan) = &self.config.backends.lan {
            self.lan_server = Some(LanServer::start_named(
                self.lan_store(),
                lan.port,
                &self.config.device_name,
            )?);
        }
        Ok(())
    }

    /// Apply configuration transactionally, including the active LAN listener.
    pub(crate) fn save_config(&mut self, mut config: Config) -> io::Result<()> {
        config.device_id = self.config.device_id.clone();
        config.device_name = self.config.device_name.clone();
        if config.backups.dir != self.config.backups.dir {
            if let Some(directory) = &config.backups.dir {
                std::fs::create_dir_all(directory)?;
            }
        }
        let same_port = self
            .lan_server
            .as_ref()
            .zip(config.backends.lan.as_ref())
            .is_some_and(|(server, lan)| server.port == lan.port);
        let pin_changed = self
            .config
            .backends
            .lan
            .as_ref()
            .zip(config.backends.lan.as_ref())
            .is_some_and(|(current, next)| current.pin != next.pin);
        let replacement = if self.serving && !same_port {
            config
                .backends
                .lan
                .as_ref()
                .map(|lan| LanServer::start_named(self.lan_store(), lan.port, &config.device_name))
                .transpose()?
        } else {
            None
        };
        if let (Some(server), Some(lan)) = (&replacement, config.backends.lan.as_mut()) {
            lan.port = server.port;
        }
        let rebuilt = Runtime::new(config.clone(), self.state_dir.clone())?;
        let path = self
            .state_dir
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("config.json");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        config.save(&path)?;
        config = Config::load(&path)?
            .ok_or_else(|| io::Error::other("saved configuration unavailable"))?;
        if self.serving {
            if same_port && pin_changed {
                self.lan_server
                    .as_ref()
                    .unwrap()
                    .set_pin(&config.backends.lan.as_ref().unwrap().pin);
            } else if !same_port {
                self.lan_server = replacement;
            }
        }
        self.config = config;
        self.base_manifests = rebuilt.base_manifests;
        self.refresh_catalog();
        Ok(())
    }
}
