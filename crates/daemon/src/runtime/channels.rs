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
                Box::new(YandexDisk::new(&ya.token)) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(gg) = &self.config.backends.google {
            out.push(Engine::new(
                Box::new(GoogleDrive::new(&gg.token)) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
        }
        if let Some(lan) = &self.config.backends.lan {
            // Своё LAN-хранилище — канал для локального движка…
            out.push(Engine::new(
                Box::new(FsBackend::new(self.lan_store())) as Box<dyn Backend>,
                self.config.device_id.clone(),
                self.state_dir.clone(),
            ));
            // …и каждый пир — отдельный канал.
            for peer in &lan.peers {
                out.push(Engine::new(
                    Box::new(LanBackend::new(peer, &lan.pin)) as Box<dyn Backend>,
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
        Ok(out)
    }

    /// Локальное хранилище LAN-канала: его обслуживает TCP-сервер
    /// (см. serve в main), а локальный движок пишет в него напрямую.
    pub fn lan_store(&self) -> PathBuf {
        self.state_dir.join("lan-store")
    }
}
