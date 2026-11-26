use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use savesync_core::config::Config;
use savesync_core::ipc::{HistoryPayload, Method, ResponsePayload, StatusPayload};
use savesync_core::lan::{DiscoveredPeer, LanAction, LanPayload};

use crate::{
    config_form::ConfigForm,
    daemon_client::{DaemonClient, IpcDaemonClient},
    presentation::*,
    theme::*,
};
mod game_details;
mod games;
mod settings;
#[cfg(test)]
mod tests;
mod updates;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
struct JobDone {
    history: Option<HistoryPayload>,
    lan: Option<LanPayload>,
    oauth_started: Option<savesync_core::oauth::Provider>,
    config: Option<Config>,
    lan_config_only: bool,
    msg: Result<String, String>,
    refresh_status: bool,
    refresh_history: bool,
}

struct Job {
    rx: mpsc::Receiver<JobDone>,
    desc: String,
    adding_game: bool,
    lan_discovery: bool,
    yandex_login: bool,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Games,
    Connections,
    Cloud,
    Backups,
    Settings,
}

pub(crate) struct SaveSyncApp {
    oauth_poll: Option<
        mpsc::Receiver<(
            savesync_core::oauth::Provider,
            Result<ResponsePayload, String>,
        )>,
    >,
    updates: updates::Updates,
    #[cfg(windows)]
    tray: Option<crate::tray::Tray>,
    #[cfg(windows)]
    allow_exit: bool,
    lan: Option<LanPayload>,
    lan_poll: Option<mpsc::Receiver<Result<ResponsePayload, String>>>,
    lan_presence_poll: Option<mpsc::Receiver<Result<ResponsePayload, String>>>,
    online_peers: Option<Vec<String>>,
    last_presence_poll: Instant,
    lan_discovered: Option<Vec<DiscoveredPeer>>,
    lan_address: String,
    lan_pair_pin: String,
    scroll_to_lan_results: bool,
    oauth_pending: Option<savesync_core::oauth::Provider>,
    yandex_client_id: String,
    yandex_notice: Option<(String, bool)>,
    yandex_check: Option<mpsc::Receiver<Result<ResponsePayload, String>>>,
    client: Arc<dyn DaemonClient>,
    tab: Tab,
    add_game: Option<games::AddGameForm>,
    daemon_ok: bool,
    status: Option<StatusPayload>,
    notice: Option<(String, bool)>, // (текст, это ошибка)
    selected: Option<String>,
    history: Option<HistoryPayload>,
    history_limit: usize,
    restore_confirmation: Option<(String, String, String)>,
    backup_picker: Option<mpsc::Receiver<Option<std::path::PathBuf>>>,
    cfg: Option<Config>,
    form: ConfigForm,
    form_loaded: bool,
    job: Option<Job>,
    last_poll: Instant,
    last_probe: Instant,
}

impl SaveSyncApp {
    pub(crate) fn new() -> Self {
        SaveSyncApp {
            oauth_poll: None,
            updates: updates::Updates::default(),
            #[cfg(windows)]
            tray: None,
            #[cfg(windows)]
            allow_exit: false,
            lan: None,
            lan_poll: None,
            lan_presence_poll: None,
            online_peers: None,
            last_presence_poll: Instant::now() - Duration::from_secs(10),
            lan_discovered: None,
            lan_address: String::new(),
            lan_pair_pin: String::new(),
            scroll_to_lan_results: false,
            oauth_pending: None,
            yandex_client_id: String::new(),
            yandex_notice: None,
            yandex_check: None,
            client: Arc::new(IpcDaemonClient),
            tab: Tab::Games,
            add_game: None,
            daemon_ok: false,
            status: None,
            notice: None,
            selected: None,
            history: None,
            history_limit: 8,
            restore_confirmation: None,
            backup_picker: None,
            cfg: None,
            form: ConfigForm::default(),
            form_loaded: false,
            job: None,
            last_poll: Instant::now() - POLL_INTERVAL,
            last_probe: Instant::now(),
        }
    }

    pub(crate) fn initialize_desktop(
        &mut self,
        context: &egui::Context,
        minimized: bool,
        startup_error: Option<String>,
        icon: &egui::IconData,
    ) {
        #[cfg(windows)]
        {
            match crate::tray::Tray::new(context, icon) {
                Ok(tray) => {
                    self.tray = Some(tray);
                    if minimized {
                        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    }
                }
                Err(error) => self.set_notice(format!("Трей недоступен: {error}"), true),
            }
        }
        #[cfg(not(windows))]
        let _ = (context, minimized, icon);
        if let Some(error) = startup_error {
            self.set_notice(error, true);
        }
    }
    #[cfg(windows)]
    fn poll_tray(&mut self, context: &egui::Context) {
        while let Some(action) = self.tray.as_ref().and_then(|tray| tray.next()) {
            match action {
                crate::tray::Action::Open => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                crate::tray::Action::Hide => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(false))
                }
                crate::tray::Action::Sync => {
                    if !self.busy() {
                        self.sync_all();
                    }
                }
                crate::tray::Action::Quit => {
                    self.allow_exit = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.tray.is_some()
            && !self.allow_exit
            && context.input(|input| input.viewport().close_requested())
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }
    fn set_notice(&mut self, text: String, is_error: bool) {
        self.notice = Some((text, is_error));
    }

    /// Фоновая операция: RPC в потоке, результат — в канал.
    fn start_job<F>(&mut self, desc: &str, method: Method, map: F)
    where
        F: FnOnce(ResponsePayload) -> (String, bool, bool) + Send + 'static,
    {
        self.start_job_with_lan_config(desc, method, None, map);
    }

    fn start_job_with_lan_config<F>(
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
                provider: savesync_core::oauth::Provider::Yandex,
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

    fn poll_status(&mut self) {
        match self.client.request(Method::Status) {
            Ok(ResponsePayload::Status(s)) => {
                self.daemon_ok = true;
                self.status = Some(s);
            }
            Ok(_) => self.daemon_ok = true,
            Err(_) => {
                self.daemon_ok = false;
                self.lan_poll = None;
                self.lan_presence_poll = None;
                self.online_peers = None;
                self.lan = None;
                self.lan_discovered = None;
                self.status = None;
                self.cfg = None;
                self.form_loaded = false;
                self.history = None;
            }
        }
    }

    fn load_history(&mut self, game_key: &str) {
        if self.job.is_some() {
            return;
        }
        self.history = None;
        self.start_job(
            "Загрузка истории…",
            Method::History {
                game_key: game_key.into(),
            },
            |_| (String::new(), false, false),
        );
    }

    fn accept_history(&mut self, history: HistoryPayload) {
        if self.selected.as_deref() == Some(history.game_key.as_str()) {
            self.history = Some(history);
        }
    }

    fn accept_config(&mut self, config: Config, lan_only: bool) {
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

    fn ensure_config(&mut self) {
        if self.cfg.is_some() || !self.daemon_ok || self.busy() {
            return;
        }
        self.start_job(
            "Загрузка настроек…",
            Method::GetConfig,
            |_| (String::new(), false, false),
        );
    }

    fn refresh_games(&mut self) {
        self.start_job("Поиск игр…", Method::Scan, |payload| {
            let message = if let ResponsePayload::Scan(scan) = payload {
                format!("найдено игр: {}", scan.found)
            } else {
                "поиск завершён".into()
            };
            (message, true, false)
        });
    }

    fn begin_oauth(&mut self, provider: savesync_core::oauth::Provider) {
        self.oauth_poll = None;
        let client_id = if provider == savesync_core::oauth::Provider::Yandex {
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
    fn cancel_oauth(&mut self, provider: savesync_core::oauth::Provider) {
        self.oauth_poll = None;
        self.start_job(
            "Отмена входа…",
            Method::OAuthCancel { provider },
            |_| ("Вход отменён".into(), false, false),
        );
        self.oauth_pending = None;
    }

    fn check_yandex_access(&mut self) {
        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        self.yandex_notice = None;
        self.yandex_check = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(client.request(Method::CloudCheck {
                provider: savesync_core::oauth::Provider::Yandex,
            }));
        });
    }

    fn collect_yandex_check(&mut self) {
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
    fn poll_oauth(&mut self) {
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
    fn collect_oauth_poll(&mut self) {
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
    fn apply_oauth_poll(
        &mut self,
        provider: savesync_core::oauth::Provider,
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
                if provider == savesync_core::oauth::Provider::Yandex {
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
                if provider == savesync_core::oauth::Provider::Yandex {
                    self.yandex_notice = Some((error.clone(), true));
                }
                self.set_notice(error, true);
            }
            _ => (),
        }
    }

    fn poll_lan(&mut self) {
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

    fn collect_lan_poll(&mut self) {
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

    fn poll_lan_presence(&mut self) {
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

    fn apply_lan_settings(&mut self, enabled: bool) {
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

    fn lan_action(&mut self, action: LanAction) {
        self.start_lan_job(action, None);
    }

    fn begin_lan_setup(&mut self, action: LanAction) {
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
    fn sync_all(&mut self) {
        self.start_job(
            "Синхронизация всех игр…",
            Method::Sync { game_key: None },
            |p| (sync_summary(&p), true, true),
        );
    }

    fn sync_game(&mut self, game_key: &str) {
        self.start_job(
            "Синхронизация…",
            Method::Sync {
                game_key: Some(game_key.to_string()),
            },
            |p| (sync_summary(&p), true, true),
        );
    }

    fn restore(&mut self, game_key: &str, version_id: &str) {
        self.start_job(
            "Восстановление сохранения…",
            Method::Restore {
                game_key: game_key.to_string(),
                version_id: version_id.to_string(),
            },
            |payload| {
                let unchanged = matches!(payload, ResponsePayload::Sync(sync)
                    if sync.results.iter().all(|result| result.outcome != "applied"));
                let message = if unchanged {
                    "Эта копия уже на устройстве"
                } else {
                    "Сохранение восстановлено"
                };
                (message.into(), true, true)
            },
        );
    }

    fn backup(&mut self, game_key: &str) {
        self.start_job(
            "Создание бэкапа…",
            Method::Backup {
                game_key: game_key.to_string(),
            },
            |p| match p {
                ResponsePayload::Backup(b) => (format!("бэкап: {}", b.file), false, false),
                _ => ("бэкап создан".to_string(), false, false),
            },
        );
    }

    fn save_config(&mut self) {
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

    fn save_backups(&mut self) {
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

    fn backup_all(&mut self) {
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

    fn busy(&self) -> bool {
        self.job.is_some() || self.status.as_ref().is_some_and(|status| status.busy)
    }
}

impl eframe::App for SaveSyncApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if crate::lifecycle::consume_exit_request() {
            #[cfg(windows)]
            {
                self.allow_exit = true;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(windows)]
        self.poll_tray(ctx);
        self.collect_oauth_poll();
        self.collect_yandex_check();
        self.updates.poll();
        ctx.request_repaint_after(Duration::from_millis(500));

        // Периодический опрос демона (быстрый локальный RPC).
        if self.job.is_none() && self.last_poll.elapsed() >= POLL_INTERVAL {
            self.poll_status();
            self.poll_oauth();
            self.ensure_config();
            self.poll_lan();
            self.last_poll = Instant::now();
        }
        self.collect_lan_poll();
        self.poll_lan_presence();

        // Завершение фоновой операции.
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

        // Верхняя панель: состояние демона и доступные устройства.
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SaveSync").size(20.0).color(ACCENT).strong());
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.daemon_ok {
                        let label = if self.status.as_ref().is_some_and(|status| status.busy) {
                            "● выполняется операция"
                        } else {
                            "● демон работает"
                        };
                        ui.label(RichText::new(label).color(OK).small());
                        if let Some(status) = &self.lan {
                            let online: Vec<_> = status
                                .peers
                                .iter()
                                .filter(|peer| {
                                    peer.confirmed
                                        && self
                                            .online_peers
                                            .as_ref()
                                            .is_some_and(|ids| ids.contains(&peer.peer.id))
                                })
                                .collect();
                            let connection = match online.as_slice() {
                                [peer] => Some(format!(
                                    "{} в сети",
                                    peer.peer.name.as_deref().unwrap_or("Устройство")
                                )),
                                [] => None,
                                _ => Some(format!("Устройств в сети: {}", online.len())),
                            };
                            if let Some(connection) = connection {
                                if ui.link(RichText::new(connection).color(OK)).clicked() {
                                    self.tab = Tab::Connections;
                                    self.poll_lan();
                                }
                            }
                        }
                    } else {
                        ui.label(RichText::new("● демон не запущен").color(WARN).small());
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::SidePanel::left("navigation")
            .exact_width(150.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                for (tab, label) in [
                    (Tab::Games, "Игры"),
                    (Tab::Connections, "Устройства"),
                    (Tab::Cloud, "Облако"),
                    (Tab::Backups, "Бэкапы"),
                    (Tab::Settings, "Настройки"),
                ] {
                    if ui
                        .add_sized(
                            [ui.available_width(), 33.0],
                            egui::SelectableLabel::new(self.tab == tab, label),
                        )
                        .clicked()
                    {
                        self.tab = tab;
                        if tab == Tab::Connections {
                            self.poll_lan();
                        }
                    }
                    ui.add_space(3.0);
                }
            });

        // Нижняя панель: уведомления / прогресс.
        egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(job) = &self.job {
                    ui.spinner();
                    ui.label(&job.desc);
                } else if let Some((msg, is_err)) = &self.notice {
                    let color = if *is_err { WARN } else { OK };
                    if msg.lines().count() > 1 {
                        ui.menu_button("Подробности", |ui| {
                            ui.set_max_width(520.0);
                            egui::ScrollArea::vertical()
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    ui.add(egui::Label::new(msg).wrap());
                                });
                        });
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(msg.lines().next().unwrap_or(msg))
                                .color(color)
                                .small(),
                        )
                        .truncate(),
                    )
                    .on_hover_text(msg);
                } else {
                    ui.label(RichText::new("готов").color(DIM).small());
                }
            });
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.daemon_ok && self.tab != Tab::Settings {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Демон не запущен").size(18.0).color(WARN));
                    ui.add_space(8.0);
                    if ui.button("Запустить SaveSync").clicked() {
                        if let Err(error) = crate::lifecycle::start_daemon() {
                            self.set_notice(error.to_string(), true);
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Приложение подключится автоматически.").color(DIM));
                });
                if self.last_probe.elapsed() >= Duration::from_secs(3) {
                    self.poll_status();
                    self.last_probe = Instant::now();
                }
                return;
            }
            match self.tab {
                Tab::Games => self.view_games(ui),
                Tab::Connections => {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| self.view_connections(ui))
                        .inner
                }
                Tab::Cloud => self.view_cloud(ui),
                Tab::Backups => self.view_backups(ui),
                Tab::Settings => self.view_settings(ui),
            }
        });
    }
}
