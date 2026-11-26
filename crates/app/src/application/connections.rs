use std::sync::mpsc;
use std::time::{Duration, Instant};

use savesync_core::backends::lan::LanAction;
use savesync_core::config::Config;
use savesync_core::ipc::{Method, ResponsePayload};

use super::{Application, Tab};

impl Application {
    pub(crate) fn begin_oauth(&mut self, provider: savesync_core::auth::oauth::Provider) {
        self.oauth_poll = None;
        let client_id = if provider == savesync_core::auth::oauth::Provider::Yandex {
            self.yandex_notice = None;
            let value = self.yandex_client_id.trim();
            (!value.is_empty()).then(|| value.to_owned())
        } else {
            None
        };
        self.start_job(
            "Подключение облака…",
            Method::OAuthBegin {
                provider,
                client_id,
            },
            |_| ("Завершите вход в системном браузере".into(), false, false),
        );
    }
    pub(crate) fn cancel_oauth(&mut self, provider: savesync_core::auth::oauth::Provider) {
        self.oauth_poll = None;
        self.start_job(
            "Отмена входа…",
            Method::OAuthCancel { provider },
            |_| ("Вход отменён".into(), false, false),
        );
        self.oauth_pending = None;
    }

    pub(crate) fn check_yandex_access(&mut self) {
        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.yandex_notice = None;
        self.yandex_check = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(client.request(Method::CloudCheck {
                provider: savesync_core::auth::oauth::Provider::Yandex,
            }));
        });
    }

    pub(super) fn collect_yandex_check(&mut self) {
        let Some(receiver) = &self.yandex_check else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(Ok(ResponsePayload::OAuthStatus(status))) if status.state == "connected" => (
                "Готов к синхронизации. Чтение и запись проверены.".into(),
                false,
            ),
            Ok(Err(error)) => (error, true),
            Err(mpsc::TryRecvError::Empty) => return,
            _ => ("Не удалось проверить доступ к Яндекс Диску".into(), true),
        };
        self.yandex_check = None;
        self.yandex_notice = Some(outcome);
    }
    pub(super) fn poll_oauth(&mut self) {
        if self.oauth_poll.is_some() || self.status.as_ref().is_some_and(|status| status.busy) {
            return;
        }
        let Some(provider) = self.oauth_pending else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.oauth_poll = Some(receiver);
        std::thread::spawn(move || {
            let result = client.request(Method::OAuthPoll { provider });
            let _ = sender.send((provider, result));
        });
    }
    pub(super) fn collect_oauth_poll(&mut self) {
        let result = match self.oauth_poll.as_ref().map(|receiver| receiver.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.oauth_poll = None;
                return;
            }
            _ => return,
        };
        self.oauth_poll = None;
        self.apply_oauth_poll(result.0, result.1);
    }
    pub(super) fn apply_oauth_poll(
        &mut self,
        provider: savesync_core::auth::oauth::Provider,
        result: Result<ResponsePayload, String>,
    ) {
        if self.oauth_pending != Some(provider) {
            return;
        }
        match result {
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "connected" => {
                self.oauth_pending = None;
                self.cfg = None;
                self.form_loaded = false;
                self.set_notice("Облако подключено".into(), false);
                if provider == savesync_core::auth::oauth::Provider::Yandex {
                    self.yandex_notice = Some((
                        "Готов к синхронизации. Чтение и запись проверены.".into(),
                        false,
                    ));
                }
            }
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "idle" => {
                self.oauth_pending = None
            }
            Err(error) => {
                self.oauth_pending = None;
                if provider == savesync_core::auth::oauth::Provider::Yandex {
                    self.yandex_notice = Some((error.clone(), true));
                }
                self.set_notice(error, true);
            }
            _ => (),
        }
    }

    pub(crate) fn poll_lan(&mut self) {
        if !self.daemon_ok
            || self.busy()
            || self.lan_poll.is_some()
            || (self.tab != Tab::Connections
                && self
                    .cfg
                    .as_ref()
                    .is_none_or(|config| config.backends.lan.is_none()))
        {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.lan_poll = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(client.request(Method::Lan {
                action: LanAction::Status,
            }));
        });
    }

    pub(super) fn collect_lan_poll(&mut self) {
        match self.lan_poll.as_ref().map(mpsc::Receiver::try_recv) {
            Some(Ok(result)) => {
                self.lan_poll = None;
                match result {
                    Ok(ResponsePayload::Lan(status)) => self.lan = Some(status),
                    Err(error) if self.lan.is_none() => self.set_notice(error, true),
                    _ => (),
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => self.lan_poll = None,
            _ => (),
        }
    }

    pub(super) fn poll_lan_presence(&mut self) {
        match self
            .lan_presence_poll
            .as_ref()
            .map(mpsc::Receiver::try_recv)
        {
            Some(Ok(Ok(ResponsePayload::LanPresence(online)))) => {
                self.lan_presence_poll = None;
                self.online_peers = Some(online);
            }
            Some(Ok(_)) | Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.lan_presence_poll = None;
                self.online_peers = None;
            }
            _ => (),
        }
        if !self.daemon_ok
            || self
                .cfg
                .as_ref()
                .is_none_or(|config| config.backends.lan.is_none())
        {
            self.lan_presence_poll = None;
            self.online_peers = None;
            return;
        }
        if self.lan_presence_poll.is_some()
            || self.last_presence_poll.elapsed() < Duration::from_secs(10)
            || !self
                .lan
                .as_ref()
                .is_some_and(|status| status.peers.iter().any(|peer| peer.confirmed))
        {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.lan_presence_poll = Some(receiver);
        self.last_presence_poll = Instant::now();
        let client = self.client.clone();
        std::thread::spawn(move || {
            let _ = sender.send(client.request(Method::LanPresence));
        });
    }

    pub(crate) fn apply_lan_settings(&mut self, enabled: bool) {
        let Some(mut config) = self.cfg.clone() else {
            return;
        };
        if let Err(error) = self.form.apply_lan_to(&mut config, enabled) {
            self.set_notice(error, true);
            return;
        }
        let changed = self.cfg.as_ref().unwrap().backends.lan != config.backends.lan;
        self.start_lan_job(LanAction::Status, changed.then_some(config));
    }

    pub(crate) fn lan_action(&mut self, action: LanAction) {
        self.start_lan_job(action, None);
    }

    pub(crate) fn begin_lan_setup(&mut self, action: LanAction) {
        let Some(mut config) = self.cfg.clone() else {
            return;
        };
        let enable = config.backends.lan.is_none();
        if enable {
            if let Err(error) = self.form.apply_lan_to(&mut config, true) {
                self.set_notice(error, true);
                return;
            }
        }
        self.start_lan_job(action, enable.then_some(config));
    }

    fn start_lan_job(&mut self, action: LanAction, config: Option<Config>) {
        self.lan_poll = None;
        self.notice = None;
        let (progress, success) = match action {
            LanAction::Discover => {
                self.lan_discovered = None;
                ("Поиск устройств в локальной сети…", "")
            }
            LanAction::Pair { .. } => (
                "Подключение к устройству…",
                "Сравните код проверки на обоих устройствах",
            ),
            LanAction::Confirm { .. } => ("Сохранение доверия…", "Устройство добавлено"),
            LanAction::Forget { .. } => ("Удаление устройства…", "Устройство удалено"),
            LanAction::Reject { .. } => ("Отмена сопряжения…", "Сопряжение отклонено"),
            LanAction::Status if config.is_some() => {
                ("Сохранение настроек LAN…", "Настройки LAN сохранены")
            }
            _ => ("Настройка LAN…", ""),
        };
        self.start_job_with_lan_config(progress, Method::Lan { action }, config, move |_| {
            (success.into(), true, false)
        });
    }
}
