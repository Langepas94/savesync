use super::*;
use savesync_core::lan::LanServer;
impl Runtime {
    pub fn get_config(&self) -> ResponsePayload {
        ResponsePayload::Config(savesync_core::ipc::ConfigPayload {
            config: self.config.clone(),
        })
    }

    pub fn start_lan(&mut self) -> io::Result<()> {
        self.serving = true;
        if let Some(lan) = &self.config.backends.lan {
            self.lan_server = Some(LanServer::start(self.lan_store(), lan.port, &lan.pin)?);
        }
        Ok(())
    }

    /// Apply configuration transactionally, including the active LAN listener.
    fn apply_config(&mut self, mut config: Config) -> io::Result<()> {
        config.device_id = self.config.device_id.clone();
        config.device_name = self.config.device_name.clone();
        let same_port = self
            .lan_server
            .as_ref()
            .zip(config.backends.lan.as_ref())
            .is_some_and(|(server, lan)| server.port == lan.port);
        let replacement = if self.serving && !same_port {
            config
                .backends
                .lan
                .as_ref()
                .map(|lan| LanServer::start(self.lan_store(), lan.port, &lan.pin))
                .transpose()?
        } else {
            None
        };
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
        if self.serving {
            if same_port {
                self.lan_server
                    .as_ref()
                    .unwrap()
                    .set_pin(&config.backends.lan.as_ref().unwrap().pin);
            } else {
                self.lan_server = replacement;
            }
        }
        self.config = config;
        self.manifests = rebuilt.manifests;
        self.refresh_catalog();
        Ok(())
    }

    pub fn save_config(&mut self, config: Config) -> ResponsePayload {
        match self.apply_config(config) {
            Ok(()) => self.get_config(),
            Err(e) => ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: "config".into(),
                    outcome: "error".into(),
                    detail: Some(e.to_string()),
                }],
            }),
        }
    }
}
