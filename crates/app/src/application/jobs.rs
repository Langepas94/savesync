use std::sync::mpsc;

use savesync_core::backends::lan::{LanAction, LanPayload};
use savesync_core::config::Config;
use savesync_core::ipc::{HistoryPayload, Method, ResponsePayload};

use super::Application;

pub(super) struct JobDone {
    history: Option<HistoryPayload>,
    lan: Option<LanPayload>,
    oauth_started: Option<savesync_core::auth::oauth::Provider>,
    pub(super) config: Option<Config>,
    pub(super) lan_config_only: bool,
    pub(super) msg: Result<String, String>,
    pub(super) refresh_status: bool,
    refresh_history: bool,
}

pub(super) struct Job {
    pub(super) rx: mpsc::Receiver<JobDone>,
    desc: String,
    adding_game: bool,
    lan_discovery: bool,
    yandex_login: bool,
}

impl Application {
    pub(crate) fn job_description(&self) -> Option<&str> {
        self.job.as_ref().map(|job| job.desc.as_str())
    }

    pub(crate) fn adding_game(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.adding_game)
    }

    /// Фоновая операция: RPC в потоке, результат — в канал.
    pub(super) fn start_job<F>(&mut self, desc: &str, method: Method, map: F)
    where
        F: FnOnce(ResponsePayload) -> (String, bool, bool) + Send + 'static,
    {
        self.start_job_with_lan_config(desc, method, None, map);
    }

    pub(super) fn start_job_with_lan_config<F>(
        &mut self,
        desc: &str,
        method: Method,
        lan_config: Option<Config>,
        map: F,
    ) where
        F: FnOnce(ResponsePayload) -> (String, bool, bool) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let client = self.client.clone();
        let adding_game = matches!(&method, Method::AddCustomGame { .. });
        let loading_history = matches!(&method, Method::History { .. });
        let yandex_login = matches!(
            &method,
            Method::OAuthBegin {
                provider: savesync_core::auth::oauth::Provider::Yandex,
                ..
            }
        );
        let lan_discovery = matches!(
            &method,
            Method::Lan {
                action: LanAction::Discover
            }
        );
        std::thread::spawn(move || {
            let lan_config_only = matches!(&method, Method::Lan { .. });
            let mut saved_config = None;
            let prepared = if let Some(config) = lan_config {
                client
                    .request(Method::SaveConfig {
                        config: Box::new(config),
                    })
                    .and_then(|payload| match payload {
                        ResponsePayload::Config(response) => {
                            saved_config = Some(response.config);
                            Ok(())
                        }
                        _ => Err("Демон не подтвердил включение LAN".into()),
                    })
            } else {
                Ok(())
            };
            let refresh_lan_config = matches!(
                &method,
                Method::Lan {
                    action: LanAction::Confirm { .. } | LanAction::Forget { .. }
                }
            );
            let response = prepared
                .and_then(|()| client.request(method))
                .and_then(|payload| {
                    if let ResponsePayload::OAuthStart(start) = &payload {
                        let url = url::Url::parse(&start.authorization_url)
                            .map_err(|_| "Некорректный OAuth URL".to_string())?;
                        if url.scheme() != "https"
                            || !matches!(
                                url.host_str(),
                                Some("accounts.google.com" | "oauth.yandex.ru")
                            )
                        {
                            return Err("Недопустимый OAuth URL".into());
                        }
                        if webbrowser::open(&start.authorization_url).is_err() {
                            let _ = client.request(Method::OAuthCancel {
                                provider: start.provider,
                            });
                            return Err(
                                "Не удалось открыть браузер; повторите remote login из CLI".into(),
                            );
                        }
                    }
                    Ok(payload)
                });
            let done = match response {
                Ok(payload) => {
                    let oauth_started = if let ResponsePayload::OAuthStart(start) = &payload {
                        Some(start.provider)
                    } else {
                        None
                    };
                    let history = if let ResponsePayload::History(history) = &payload {
                        Some(history.clone())
                    } else {
                        None
                    };
                    let lan = if let ResponsePayload::Lan(status) = &payload {
                        Some(status.clone())
                    } else {
                        None
                    };
                    let mut config_error = None;
                    let config = if refresh_lan_config {
                        match client.request(Method::GetConfig) {
                            Ok(ResponsePayload::Config(response)) => Some(response.config),
                            result => {
                                config_error = Some(format!(
                                    "Настройки LAN не обновлены: {}",
                                    match result {
                                        Err(error) => error,
                                        _ => "неожиданный ответ демона".into(),
                                    }
                                ));
                                None
                            }
                        }
                    } else if let ResponsePayload::Config(response) = &payload {
                        Some(response.config.clone())
                    } else {
                        saved_config.clone()
                    };
                    let (msg, rs, rh) = map(payload);
                    JobDone {
                        history,
                        lan,
                        oauth_started,
                        config,
                        lan_config_only,
                        msg: config_error.map_or(Ok(msg), Err),
                        refresh_status: rs,
                        refresh_history: rh,
                    }
                }
                Err(e) => JobDone {
                    history: None,
                    lan: None,
                    oauth_started: None,
                    config: saved_config,
                    lan_config_only,
                    msg: Err(e),
                    refresh_status: true,
                    refresh_history: !loading_history,
                },
            };
            let _ = tx.send(done);
        });
        self.job = Some(Job {
            rx,
            desc: desc.to_string(),
            adding_game,
            lan_discovery,
            yandex_login,
        });
    }

    pub(super) fn finish_job(&mut self) {
        let job_done = match self.job.as_ref().map(|job| (job.rx.try_recv(), job)) {
            Some((Ok(done), job)) => {
                Some((done, job.adding_game, job.lan_discovery, job.yandex_login))
            }
            Some((Err(mpsc::TryRecvError::Disconnected), _)) => {
                self.job = None;
                self.set_notice(
                    "Фоновая операция завершилась без ответа. Попробуйте снова.".into(),
                    true,
                );
                None
            }
            _ => None,
        };
        if let Some((done, adding_game, lan_discovery, yandex_login)) = job_done {
            self.job = None;
            if yandex_login {
                if let Err(error) = &done.msg {
                    self.yandex_notice = Some((error.clone(), true));
                }
            }
            if adding_game {
                match &done.msg {
                    Ok(_) => self.add_game = None,
                    Err(error) => {
                        if let Some(form) = &mut self.add_game {
                            form.error = Some(error.clone());
                        }
                    }
                }
            }
            if let Some(provider) = done.oauth_started {
                self.oauth_pending = Some(provider);
            }
            if let Some(history) = done.history {
                self.accept_history(history);
            }
            let lan_only = done.lan_config_only;
            if done.lan.is_some() && done.config.is_none() && done.msg.is_err() {
                self.cfg = None;
                self.form_loaded = false;
            }
            if let Some(mut lan) = done.lan {
                if lan.offer.is_some() {
                    self.lan_pair_pin.clear();
                }
                if lan_discovery {
                    self.scroll_to_lan_results = !lan.discovered.is_empty();
                    self.lan_discovered = Some(std::mem::take(&mut lan.discovered));
                }
                self.lan = Some(lan);
            }
            if let Some(config) = done.config {
                self.accept_config(config, lan_only);
            }
            match done.msg {
                Ok(m) if !m.is_empty() => self.set_notice(m, false),
                Ok(_) => (),
                Err(e) => self.set_notice(e, true),
            }
            if done.refresh_status {
                self.poll_status();
            }
            if done.refresh_history {
                if let Some(key) = self.selected.clone() {
                    self.load_history(&key);
                }
            }
        }
    }
}
