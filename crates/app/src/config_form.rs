use savesync_core::config::{Config, GoogleConfig, LanConfig, YandexConfig};
/// Редактируемая копия конфига (всё строками; пустое = None).
#[derive(Default)]
pub(crate) struct ConfigForm {
    pub(crate) fs_path: String,
    pub(crate) yandex_token: String,
    pub(crate) google_token: String,
    pub(crate) lan_enabled: bool,
    pub(crate) lan_port: String,
    pub(crate) lan_pin: String,
    pub(crate) lan_peers: String,
    pub(crate) backups_dir: String,
    pub(crate) backups_keep: String,
    pub(crate) backups_auto_hours: String,
}

impl ConfigForm {
    pub(crate) fn from_config(c: &Config) -> Self {
        ConfigForm {
            fs_path: c
                .backends
                .fs_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            yandex_token: c
                .backends
                .yandex
                .as_ref()
                .map(|y| y.token.clone())
                .unwrap_or_default(),
            google_token: c
                .backends
                .google
                .as_ref()
                .map(|g| g.token.clone())
                .unwrap_or_default(),
            lan_enabled: c.backends.lan.is_some(),
            lan_port: c
                .backends
                .lan
                .as_ref()
                .map(|l| l.port.to_string())
                .unwrap_or_else(|| "47660".into()),
            lan_pin: c
                .backends
                .lan
                .as_ref()
                .map(|l| l.pin.clone())
                .unwrap_or_default(),
            lan_peers: c
                .backends
                .lan
                .as_ref()
                .map(|l| l.peers.join(", "))
                .unwrap_or_default(),
            backups_dir: c
                .backups
                .dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            backups_keep: c.backups.keep.map(|k| k.to_string()).unwrap_or_default(),
            backups_auto_hours: c
                .backups
                .auto_interval_hours
                .map(|h| h.to_string())
                .unwrap_or_default(),
        }
    }

    /// Переносит поля формы в конфиг; device_id/name не трогает.
    pub(crate) fn apply_to(&self, c: &mut Config) -> Result<(), String> {
        let trimmed = |s: &str| s.trim().to_string();
        c.backends.fs_path = (!trimmed(&self.fs_path).is_empty())
            .then(|| std::path::PathBuf::from(trimmed(&self.fs_path)));
        c.backends.yandex = (!trimmed(&self.yandex_token).is_empty()).then(|| YandexConfig {
            token: trimmed(&self.yandex_token),
        });
        c.backends.google = (!trimmed(&self.google_token).is_empty()).then(|| GoogleConfig {
            token: trimmed(&self.google_token),
        });
        self.apply_lan_to(c)?;
        c.backups.dir = (!trimmed(&self.backups_dir).is_empty())
            .then(|| std::path::PathBuf::from(trimmed(&self.backups_dir)));
        c.backups.keep = if trimmed(&self.backups_keep).is_empty() {
            None
        } else {
            Some(
                trimmed(&self.backups_keep)
                    .parse()
                    .map_err(|_| "Бэкапы: keep должно быть числом".to_string())?,
            )
        };
        c.backups.auto_interval_hours = if trimmed(&self.backups_auto_hours).is_empty() {
            None
        } else {
            Some(
                trimmed(&self.backups_auto_hours)
                    .parse()
                    .map_err(|_| "Бэкапы: интервал должен быть числом часов".to_string())?,
            )
        };
        Ok(())
    }

    pub(crate) fn apply_lan_to(&self, c: &mut Config) -> Result<(), String> {
        let trimmed = |s: &str| s.trim().to_string();
        c.backends.lan = if self.lan_enabled {
            let port: u16 = trimmed(&self.lan_port)
                .parse()
                .map_err(|_| "LAN: порт должен быть числом 1-65535".to_string())?;
            let peers = trimmed(&self.lan_peers)
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some(LanConfig {
                port,
                pin: trimmed(&self.lan_pin),
                peers,
            })
        } else {
            None
        };
        Ok(())
    }
}
