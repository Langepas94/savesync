use super::*;
impl Runtime {
    /// Каналы синхронизации (plan §3): настроенные бэкенды, каждый —
    /// отдельный Engine. Каналы независимы: sync идёт по всем.
    pub fn engines(&self) -> Result<Vec<Engine<Box<dyn Backend>>>, String> {
        let mut out: Vec<Engine<Box<dyn Backend>>> = Vec::new();
        if let Some(fs) = &self.config.backends.fs_path {
            out.push(Engine::new(
                Box::new(FsBackend::new(fs)) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(ya) = &self.config.backends.yandex {
            out.push(Engine::new(
                savesync_core::credential_backend::CredentialBackend::connect(
                    savesync_core::oauth::Provider::Yandex,
                    &ya.token,
                ),
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(gg) = &self.config.backends.google {
            out.push(Engine::new(
                savesync_core::credential_backend::CredentialBackend::connect(
                    savesync_core::oauth::Provider::Google,
                    &gg.token,
                ),
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(lan) = &self.config.backends.lan {
            // Своё LAN-хранилище — канал для локального движка…
            out.push(
                Engine::new(
                    Box::new(FsBackend::new(self.lan_store())) as Box<dyn Backend>,
                    self.config.device_id.clone(),
                    self.state_dir.clone(),
                )
                .with_history_channel("lan"),
            );
            // …и каждый пир — отдельный канал.
            let identity = self.state_dir.join("lan-identity");
            let mut addresses = lan.peers.clone();
            for address in savesync_core::lan::peers(&identity)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|entry| entry.confirmed)
                .filter_map(|entry| entry.peer.address)
            {
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
            for peer in &addresses {
                out.push(Engine::new(
                    Box::new(LanBackend::with_identity(peer, &identity)) as Box<dyn Backend>,
                    self.config.device_id.clone(),
                    self.state_dir.clone(),
                ));
            }
        }
        if out.is_empty() {
            return Err(
                "канал синхронизации не настроен (backends.fs_path / yandex / google / lan)".into(),
            );
        }
        let fallback;
        let name = if self.config.device_name == "device" {
            fallback = savesync_core::config::hostname();
            &fallback
        } else {
            &self.config.device_name
        };
        Ok(out
            .into_iter()
            .map(|engine| engine.with_device_name(name))
            .collect())
    }

    /// Локальное хранилище LAN-канала: его обслуживает TCP-сервер
    /// (см. serve в main), а локальный движок пишет в него напрямую.
    pub fn lan_store(&self) -> PathBuf {
        self.state_dir.join("lan-store")
    }
}
