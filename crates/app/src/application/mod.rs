use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use savesync_core::config::Config;
use savesync_core::ipc::{HistoryPayload, Method, ResponsePayload, StatusPayload};

use crate::{
    config_form::ConfigForm,
    daemon_client::{DaemonClient, IpcDaemonClient},
    presentation::*,
    theme::*,
};
mod games;
mod settings;
#[cfg(test)]
mod tests;
mod updates;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
struct JobDone {
    history: Option<HistoryPayload>,
    lan: Option<savesync_core::lan::LanPayload>,
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
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Games,
    Connections,
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
    lan: Option<savesync_core::lan::LanPayload>,
    lan_address: String,
    lan_pair_pin: String,
    oauth_pending: Option<savesync_core::oauth::Provider>,
    client: Arc<dyn DaemonClient>,
    tab: Tab,
    add_game: Option<games::AddGameForm>,
    daemon_ok: bool,
    status: Option<StatusPayload>,
    notice: Option<(String, bool)>, // (текст, это ошибка)
    selected: Option<String>,
    history: Option<HistoryPayload>,
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
            lan_address: String::new(),
            lan_pair_pin: String::new(),
            oauth_pending: None,
            client: Arc::new(IpcDaemonClient),
            tab: Tab::Games,
            add_game: None,
            daemon_ok: false,
            status: None,
            notice: None,
            selected: None,
            history: None,
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
                    action: savesync_core::lan::LanAction::Confirm { .. }
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
                                    "Доверие подтверждено, но настройки LAN не обновлены: {}",
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
                    refresh_history: true,
                },
            };
            let _ = tx.send(done);
        });
        self.job = Some(Job {
            rx,
            desc: desc.to_string(),
            adding_game,
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
            self.form.lan_enabled = form.lan_enabled;
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
        self.start_job(
            "Подключение облака…",
            Method::OAuthBegin { provider },
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
            }
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "idle" => {
                self.oauth_pending = None
            }
            Err(error) => {
                self.oauth_pending = None;
                self.set_notice(error, true);
            }
            _ => (),
        }
    }

    fn lan_action(&mut self, action: savesync_core::lan::LanAction) {
        let Some(mut config) = self.cfg.clone() else {
            self.set_notice(
                "Настройки ещё не загружены; дождитесь подключения к демону".into(),
                true,
            );
            return;
        };
        if let Err(error) = self.form.apply_lan_to(&mut config) {
            self.set_notice(error, true);
            return;
        }
        let changed = self.cfg.as_ref().unwrap().backends.lan != config.backends.lan;
        let discover = matches!(&action, savesync_core::lan::LanAction::Discover);
        self.start_job_with_lan_config("LAN…", Method::Lan { action }, changed.then_some(config), move |payload| {
            let message = match payload {
                ResponsePayload::Lan(status) if discover && status.discovered.is_empty() =>
                    "Устройства не найдены. Включите LAN на втором устройстве; проверьте общую сеть или введите IPv4:порт вручную.".into(),
                ResponsePayload::Lan(status) if discover => format!("Найдено адресов устройств: {}", status.discovered.len()),
                _ => "Состояние LAN обновлено; для сопряжения сравните SAS на обоих устройствах".into(),
            };
            (message, true, false)
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
        let desc = "Применение версии…".to_string();
        self.start_job(
            &desc,
            Method::Restore {
                game_key: game_key.to_string(),
                version_id: version_id.to_string(),
            },
            |_| ("версия применена".to_string(), true, true),
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
        match self.form.apply_to(&mut cfg) {
            Ok(()) => {
                self.set_notice("сохранение конфига…".into(), false);
                self.start_job(
                    "Сохранение конфига…",
                    Method::SaveConfig {
                        config: Box::new(cfg),
                    },
                    |_| ("конфиг сохранён".to_string(), true, false),
                );
            }
            Err(e) => self.set_notice(e, true),
        }
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
        self.updates.poll();
        ctx.request_repaint_after(Duration::from_millis(500));

        // Периодический опрос демона (быстрый локальный RPC).
        if self.job.is_none() && self.last_poll.elapsed() >= POLL_INTERVAL {
            self.poll_status();
            self.poll_oauth();
            self.ensure_config();
            self.last_poll = Instant::now();
        }

        // Завершение фоновой операции.
        let mut job_done = None;
        if let Some(job) = &self.job {
            if let Ok(done) = job.rx.try_recv() {
                job_done = Some((done, job.adding_game));
            }
        }
        if let Some((done, adding_game)) = job_done {
            self.job = None;
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
            if let Some(lan) = done.lan {
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

        // Верхняя панель: заголовок + вкладки + статус демона.
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SaveSync").size(20.0).color(ACCENT).strong());
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Games, "Игры");
                ui.selectable_value(&mut self.tab, Tab::Connections, "Подключения");
                ui.selectable_value(&mut self.tab, Tab::Settings, "Настройки");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.daemon_ok {
                        let label = if self.status.as_ref().is_some_and(|status| status.busy) {
                            "● выполняется операция"
                        } else {
                            "● демон работает"
                        };
                        ui.label(RichText::new(label).color(OK).small());
                    } else {
                        ui.label(RichText::new("● демон не запущен").color(WARN).small());
                    }
                });
            });
            ui.add_space(4.0);
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
                    for (i, line) in msg.lines().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        ui.label(RichText::new(line).color(color).small());
                    }
                } else {
                    ui.label(RichText::new("готов").color(DIM).small());
                }
            });
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.daemon_ok {
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
                Tab::Connections => self.view_connections(ui),
                Tab::Settings => self.view_settings(ui),
            }
        });
    }
}
