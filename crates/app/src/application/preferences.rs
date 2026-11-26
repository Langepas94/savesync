use savesync_core::config::Config;
use savesync_core::ipc::{Method, ResponsePayload};

use super::{Application, ConfigForm};

impl Application {
    pub(super) fn accept_config(&mut self, config: Config, lan_only: bool) {
        if lan_only {
            let form = ConfigForm::from_config(&config);
            self.form.lan_port = form.lan_port;
            self.form.lan_pin = form.lan_pin;
            self.form.lan_peers = form.lan_peers;
            if let Some(current) = &mut self.cfg {
                current.backends.lan = config.backends.lan;
            }
        } else {
            self.form = ConfigForm::from_config(&config);
            self.cfg = Some(config);
            self.form_loaded = true;
        }
    }

    pub(super) fn ensure_config(&mut self) {
        if self.cfg.is_some() || !self.daemon_ok || self.busy() {
            return;
        }
        self.start_job(
            "Загрузка настроек…",
            Method::GetConfig,
            |_| (String::new(), false, false),
        );
    }

    pub(crate) fn save_config(&mut self) {
        let Some(mut cfg) = self.cfg.clone() else {
            return;
        };
        self.form.apply_storage_to(&mut cfg);
        if self.cfg.as_ref().and_then(|c| c.backends.yandex.as_ref())
            != cfg.backends.yandex.as_ref()
        {
            self.yandex_notice = None;
        }
        self.start_job(
            "Сохранение настроек каналов…",
            Method::SaveConfig {
                config: Box::new(cfg),
            },
            |_| ("Настройки каналов сохранены".into(), true, false),
        );
    }

    pub(crate) fn save_backups(&mut self) {
        let Some(mut cfg) = self.cfg.clone() else {
            return;
        };
        if let Err(error) = self.form.apply_backups_to(&mut cfg) {
            self.set_notice(error, true);
            return;
        }
        if cfg.backups.dir
            != self
                .cfg
                .as_ref()
                .and_then(|current| current.backups.dir.clone())
            && !cfg
                .backups
                .dir
                .as_ref()
                .is_some_and(|path| path.is_absolute())
        {
            self.set_notice("Выберите папку или укажите полный путь".into(), true);
            return;
        }
        self.start_job(
            "Сохранение настроек бэкапов…",
            Method::SaveConfig {
                config: Box::new(cfg),
            },
            |_| ("Настройки бэкапов сохранены".into(), false, false),
        );
    }

    pub(crate) fn backup_all(&mut self) {
        self.start_job(
            "Создание бэкапов…",
            Method::BackupAll,
            |payload| {
                let message = match payload {
                    ResponsePayload::BackupBatch(batch) if batch.created > 0 => {
                        format!("Создано бэкапов: {} · {}", batch.created, batch.directory)
                    }
                    _ => "Нет сохранений для резервного копирования".into(),
                };
                (message, false, false)
            },
        );
    }
}
