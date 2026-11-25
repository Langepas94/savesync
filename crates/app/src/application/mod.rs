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
    lan: Option<savesync_core::lan::LanPayload>,
    oauth_started: Option<savesync_core::oauth::Provider>,
    config: Option<Config>,
    msg: Result<String, String>,
    refresh_status: bool,
    refresh_history: bool,
}

struct Job {
    rx: mpsc::Receiver<JobDone>,
    desc: String,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Games,
    Connections,
    Settings,
}

pub(crate) struct SaveSyncApp {
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
    ) {
        #[cfg(windows)]
        {
            match crate::tray::Tray::new(context) {
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
        let _ = (context, minimized);
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
        let (tx, rx) = mpsc::channel();
        let client = self.client.clone();
        std::thread::spawn(move || {
            let response = client.request(method).and_then(|payload| {
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
                            "Не удалось открыть браузер; повторите remote login из CLI".into()
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
                    let lan = if let ResponsePayload::Lan(status) = &payload {
                        Some(status.clone())
                    } else {
                        None
                    };
                    let config = if let ResponsePayload::Config(response) = &payload {
                        Some(response.config.clone())
                    } else {
                        None
                    };
                    let (msg, rs, rh) = map(payload);
                    JobDone {
                        lan,
                        oauth_started,
                        config,
                        msg: Ok(msg),
                        refresh_status: rs,
                        refresh_history: rh,
                    }
                }
                Err(e) => JobDone {
                    lan: None,
                    oauth_started: None,
                    config: None,
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
        match self.client.request(Method::History {
            game_key: game_key.to_string(),
        }) {
            Ok(ResponsePayload::History(h)) => self.history = Some(h),
            Ok(_) => {}
            Err(e) => {
                self.history = None;
                self.set_notice(format!("история: {e}"), true);
            }
        }
    }

    fn ensure_config(&mut self) {
        if self.cfg.is_some() || !self.daemon_ok {
            return;
        }
        if let Ok(ResponsePayload::Config(p)) = self.client.request(Method::GetConfig) {
            self.form = ConfigForm::from_config(&p.config);
            self.form_loaded = true;
            self.cfg = Some(p.config);
        }
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
        self.start_job(
            "Подключение облака…",
            Method::OAuthBegin { provider },
            |_| ("Завершите вход в системном браузере".into(), false, false),
        );
    }
    fn cancel_oauth(&mut self, provider: savesync_core::oauth::Provider) {
        self.start_job(
            "Отмена входа…",
            Method::OAuthCancel { provider },
            |_| ("Вход отменён".into(), false, false),
        );
        self.oauth_pending = None;
    }
    fn poll_oauth(&mut self) {
        let Some(provider) = self.oauth_pending else {
            return;
        };
        match self.client.request(Method::OAuthPoll { provider }) {
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "connected" => {
                self.oauth_pending = None;
                self.cfg = None;
                self.form_loaded = false;
                self.set_notice("Облако подключено".into(), false);
            }
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "idle" => {
                self.oauth_pending = None;
            }
            Err(error) => {
                self.oauth_pending = None;
                self.set_notice(error, true);
            }
            _ => (),
        }
    }

    fn lan_action(&mut self, action: savesync_core::lan::LanAction) {
        self.start_job("LAN…", Method::Lan { action }, |_| {
            (
                "Состояние LAN обновлено; для сопряжения сравните SAS на обоих устройствах".into(),
                true,
                false,
            )
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
        self.job.is_some()
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
                job_done = Some(done);
            }
        }
        if let Some(done) = job_done {
            self.job = None;
            if let Some(provider) = done.oauth_started {
                self.oauth_pending = Some(provider);
            }
            if let Some(lan) = done.lan {
                self.lan = Some(lan);
            }
            if let Some(config) = done.config {
                self.form = ConfigForm::from_config(&config);
                self.cfg = Some(config);
                self.form_loaded = true;
            }
            match done.msg {
                Ok(m) => self.set_notice(m, false),
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
                        ui.label(RichText::new("● демон работает").color(OK).small());
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
                    ui.label("Запустите в терминале:");
                    ui.code("savesync-daemon serve");
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
