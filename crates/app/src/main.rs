//! SaveSync — десктопное приложение (egui/eframe).
//!
//! Клиент демона по IPC (`savesync_core::ipc::client`): статус и история
//! опрашиваются синхронно (локальный сокет, миллисекунды), длительные
//! операции (sync/restore/backup) уходят в фоновый поток с mpsc-каналом.
//!
//! Экраны по docs/design/desktop-app.html: Игры (+детали, конфликт-
//! резолвер), Подключения (каналы), Настройки (бэкапы).
//!
//! Палитра Steam: #171a21 фон, #1b2838 панели, #66c0f4 акцент.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};

use savesync_core::config::{Config, GoogleConfig, LanConfig, YandexConfig};
use savesync_core::ipc::client::{self, DEFAULT_SOCKET};
use savesync_core::ipc::{HistoryPayload, Method, Request, ResponsePayload, StatusPayload};

const BG: Color32 = Color32::from_rgb(0x17, 0x1a, 0x21);
const PANEL: Color32 = Color32::from_rgb(0x1b, 0x28, 0x38);
const ACCENT: Color32 = Color32::from_rgb(0x66, 0xc0, 0xf4);
const OK: Color32 = Color32::from_rgb(0x7c, 0xc4, 0x6c);
const WARN: Color32 = Color32::from_rgb(0xe5, 0x5a, 0x5a);
const DIM: Color32 = Color32::from_rgb(0x8f, 0x98, 0xa0);

const POLL_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 660.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SaveSync",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(SaveSyncApp::new()))
        }),
    )
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x10, 0x13, 0x18);
    v.faint_bg_color = PANEL;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT;
    ctx.set_visuals(v);
}

/// Вызов демона; ошибка сведена к строке для UI.
fn rpc(method: Method) -> Result<ResponsePayload, String> {
    let resp = client::call(DEFAULT_SOCKET, Request::new(1, method))
        .map_err(|e| format!("демон недоступен: {e}"))?;
    if let Some(err) = resp.error {
        return Err(err.message);
    }
    resp.result.ok_or_else(|| "пустой ответ демона".to_string())
}

struct JobDone {
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

/// Редактируемая копия конфига (всё строками; пустое = None).
#[derive(Default)]
struct ConfigForm {
    fs_path: String,
    yandex_token: String,
    google_token: String,
    lan_enabled: bool,
    lan_port: String,
    lan_pin: String,
    lan_peers: String,
    backups_dir: String,
    backups_keep: String,
    backups_auto_hours: String,
}

impl ConfigForm {
    fn from_config(c: &Config) -> Self {
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
    fn apply_to(&self, c: &mut Config) -> Result<(), String> {
        let trimmed = |s: &str| s.trim().to_string();
        c.backends.fs_path = (!trimmed(&self.fs_path).is_empty())
            .then(|| std::path::PathBuf::from(trimmed(&self.fs_path)));
        c.backends.yandex = (!trimmed(&self.yandex_token).is_empty()).then(|| YandexConfig {
            token: trimmed(&self.yandex_token),
        });
        c.backends.google = (!trimmed(&self.google_token).is_empty()).then(|| GoogleConfig {
            token: trimmed(&self.google_token),
        });
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
}

struct SaveSyncApp {
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
    fn new() -> Self {
        SaveSyncApp {
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

    fn set_notice(&mut self, text: String, is_error: bool) {
        self.notice = Some((text, is_error));
    }

    /// Фоновая операция: RPC в потоке, результат — в канал.
    fn start_job<F>(&mut self, desc: &str, method: Method, map: F)
    where
        F: FnOnce(ResponsePayload) -> (String, bool, bool) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let done = match rpc(method) {
                Ok(payload) => {
                    let (msg, rs, rh) = map(payload);
                    JobDone {
                        msg: Ok(msg),
                        refresh_status: rs,
                        refresh_history: rh,
                    }
                }
                Err(e) => JobDone {
                    msg: Err(e),
                    refresh_status: true,
                    refresh_history: false,
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
        match rpc(Method::Status) {
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
        match rpc(Method::History {
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
        if let Ok(ResponsePayload::Config(p)) = rpc(Method::GetConfig) {
            self.form = ConfigForm::from_config(&p.config);
            self.form_loaded = true;
            self.cfg = Some(p.config);
        }
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

fn sync_summary(p: &ResponsePayload) -> String {
    if let ResponsePayload::Sync(s) = p {
        let mut lines = Vec::new();
        for r in &s.results {
            let mut line = format!("{}: {}", r.game_key, r.outcome);
            if let Some(d) = &r.detail {
                line.push_str(&format!(" ({d})"));
            }
            lines.push(line);
        }
        if lines.is_empty() {
            "нет игр для синхронизации".to_string()
        } else {
            lines.join("\n")
        }
    } else {
        "готово".to_string()
    }
}

/// Форматирует unix-millis как "YYYY-MM-DD HH:MM" (UTC, без chrono).
fn fmt_ts(millis: u64) -> String {
    let secs = millis / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant's civil-from-days (UTC).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn state_badge(ui: &mut egui::Ui, state: &str) {
    let (text, color) = match state {
        "ok" => ("синхронизировано", OK),
        "conflict" => ("конфликт", WARN),
        "no_versions" => ("нет версий", DIM),
        other => (other, DIM),
    };
    ui.label(RichText::new(text).color(color));
}

impl eframe::App for SaveSyncApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(500));

        // Периодический опрос демона (быстрый локальный RPC).
        if self.job.is_none() && self.last_poll.elapsed() >= POLL_INTERVAL {
            self.poll_status();
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

impl SaveSyncApp {
    fn view_games(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Игры");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Синхронизировать всё"))
                    .clicked()
                {
                    self.sync_all();
                }
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Обновить"))
                    .clicked()
                {
                    self.poll_status();
                }
            });
        });
        ui.separator();

        let games = self
            .status
            .as_ref()
            .map(|s| s.games.clone())
            .unwrap_or_default();
        if games.is_empty() {
            ui.label(RichText::new("Игры не найдены. Выполните: savesync-daemon scan").color(DIM));
        }

        let mut sync_one = None;
        let mut select_one = None;
        egui::ScrollArea::both()
            .id_salt("games")
            .max_height(ui.available_height() * 0.45)
            .show(ui, |ui| {
                for g in &games {
                    let is_sel = self.selected.as_deref() == Some(g.game_key.as_str());
                    let frame = egui::Frame::group(ui.style()).fill(if is_sel {
                        Color32::from_rgb(0x24, 0x35, 0x47)
                    } else {
                        PANEL
                    });
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&g.display_name).strong());
                            ui.label(
                                RichText::new(format!("версий: {}", g.versions))
                                    .color(DIM)
                                    .small(),
                            );
                            state_badge(ui, &g.state);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(!self.busy(), egui::Button::new("Синк"))
                                        .clicked()
                                    {
                                        sync_one = Some(g.game_key.clone());
                                    }
                                    if ui
                                        .button(if is_sel {
                                            "Скрыть"
                                        } else {
                                            "Детали"
                                        })
                                        .clicked()
                                    {
                                        select_one = Some(if is_sel {
                                            None
                                        } else {
                                            Some(g.game_key.clone())
                                        });
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(2.0);
                }
            });

        if let Some(key) = sync_one {
            self.sync_game(&key);
        }
        if let Some(sel) = select_one {
            match sel {
                Some(key) => {
                    self.selected = Some(key.clone());
                    self.load_history(&key);
                }
                None => {
                    self.selected = None;
                    self.history = None;
                }
            }
        }

        ui.separator();
        self.view_detail(ui);
    }

    fn view_detail(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        ui.heading("Детали");
        ui.horizontal(|ui| {
            ui.label(RichText::new(&key).color(ACCENT));
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Сделать бэкап"))
                .clicked()
            {
                self.backup(&key);
            }
        });

        let Some(h) = self.history.clone() else {
            ui.label(RichText::new("нет истории").color(DIM));
            return;
        };

        // Конфликт-резолвер: больше одной головы.
        if h.heads.len() > 1 {
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(0x3a, 0x24, 0x24))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Конфликт: {} ветки. Выберите версию ниже — она станет общей.",
                            h.heads.len()
                        ))
                        .color(WARN),
                    );
                });
        }

        let mut restore_id = None;
        egui::ScrollArea::vertical()
            .id_salt("history")
            .show(ui, |ui| {
                for v in h.versions.iter().rev() {
                    egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let is_applied = h.applied.as_deref() == Some(v.id.as_str());
                            let is_head = h.heads.contains(&v.id);
                            ui.label(fmt_ts(v.created_at));
                            ui.label(RichText::new(&v.device).color(DIM));
                            ui.label(
                                RichText::new(&v.id[..v.id.len().min(12)])
                                    .color(DIM)
                                    .monospace()
                                    .small(),
                            );
                            if is_head {
                                ui.label(RichText::new("голова").color(WARN).small());
                            }
                            if is_applied {
                                ui.label(RichText::new("применена").color(OK).small());
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(
                                            !self.busy() && !is_applied,
                                            egui::Button::new("Применить"),
                                        )
                                        .clicked()
                                    {
                                        restore_id = Some(v.id.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(2.0);
                }
            });
        if let Some(id) = restore_id {
            self.restore(&key, &id);
        }
    }

    fn view_connections(&mut self, ui: &mut egui::Ui) {
        ui.heading("Подключения");
        ui.label(
            RichText::new(
                "Каналы независимы: можно настроить любой набор. Токены вводятся вручную.",
            )
            .color(DIM)
            .small(),
        );
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Локальная папка / USB (fs)").strong());
                ui.horizontal(|ui| {
                    ui.label("Путь:");
                    ui.add(egui::TextEdit::singleline(&mut self.form.fs_path).desired_width(420.0));
                });
            });
            ui.add_space(6.0);

            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Яндекс.Диск").strong());
                ui.horizontal(|ui| {
                    ui.label("OAuth-токен:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.yandex_token)
                            .password(true)
                            .desired_width(380.0),
                    );
                });
            });
            ui.add_space(6.0);

            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Google Drive").strong());
                ui.horizontal(|ui| {
                    ui.label("Access token:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.google_token)
                            .password(true)
                            .desired_width(380.0),
                    );
                });
            });
            ui.add_space(6.0);

            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("LAN (прямая синхронизация по сети)").strong());
                ui.checkbox(&mut self.form.lan_enabled, "включить LAN-канал");
                if self.form.lan_enabled {
                    ui.horizontal(|ui| {
                        ui.label("Порт:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_port).desired_width(80.0),
                        );
                        ui.label("PIN:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_pin)
                                .password(true)
                                .desired_width(120.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Пиры (host:port, через запятую):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_peers)
                                .desired_width(320.0),
                        );
                    });
                }
            });
            ui.add_space(10.0);

            if ui
                .add_enabled(!self.busy(), egui::Button::new("Сохранить"))
                .clicked()
            {
                self.save_config();
            }
        });
    }

    fn view_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Настройки");
        ui.add_space(8.0);

        if let Some(cfg) = &self.cfg {
            ui.horizontal(|ui| {
                ui.label("Устройство:");
                ui.label(RichText::new(&cfg.device_name).strong());
                ui.label(
                    RichText::new(format!("({})", cfg.device_id))
                        .color(DIM)
                        .small(),
                );
            });
            if let Some(s) = &self.status {
                ui.label(
                    RichText::new(format!("демон v{}", s.daemon_version))
                        .color(DIM)
                        .small(),
                );
            }
            ui.add_space(8.0);
        }

        egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
            ui.label(RichText::new("Бэкапы").strong());
            ui.horizontal(|ui| {
                ui.label("Папка бэкапов:");
                ui.add(egui::TextEdit::singleline(&mut self.form.backups_dir).desired_width(420.0));
            });
            ui.horizontal(|ui| {
                ui.label("Хранить копий на игру:");
                ui.add(egui::TextEdit::singleline(&mut self.form.backups_keep).desired_width(60.0));
                ui.label(RichText::new("пусто — по умолчанию 10").color(DIM).small());
            });
            ui.horizontal(|ui| {
                ui.label("Авто-бэкап каждые N часов:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.backups_auto_hours)
                        .desired_width(60.0),
                );
                ui.label(RichText::new("пусто — выключено").color(DIM).small());
            });
        });
        ui.add_space(10.0);

        if ui
            .add_enabled(!self.busy(), egui::Button::new("Сохранить"))
            .clicked()
        {
            self.save_config();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use savesync_core::ipc::SyncResult;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-01-01 = 20454 (2025 не високосный), +328 дней до 25 ноября.
        assert_eq!(civil_from_days(20_782), (2026, 11, 25));
    }

    #[test]
    fn fmt_ts_zero_is_epoch() {
        assert_eq!(fmt_ts(0), "1970-01-01 00:00");
    }

    #[test]
    fn form_roundtrip_preserves_tokens() {
        let cfg = Config {
            version: 1,
            device_id: "pc-12345678".into(),
            device_name: "PC".into(),
            manifests_dir: None,
            backends: savesync_core::config::BackendsConfig {
                fs_path: Some(std::path::PathBuf::from("D:/saves")),
                yandex: Some(YandexConfig {
                    token: "y0_tok".into(),
                }),
                google: None,
                lan: Some(LanConfig {
                    port: 47660,
                    pin: "1234".into(),
                    peers: vec!["deck:47660".into()],
                }),
            },
            backups: savesync_core::config::BackupsSettings {
                dir: Some(std::path::PathBuf::from("D:/backups")),
                keep: Some(5),
                auto_interval_hours: Some(24),
            },
        };
        let form = ConfigForm::from_config(&cfg);
        let mut back = cfg.clone();
        form.apply_to(&mut back).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn form_rejects_bad_port() {
        let form = ConfigForm {
            lan_enabled: true,
            lan_port: "abc".into(),
            ..Default::default()
        };
        let mut cfg = Config {
            version: 1,
            device_id: "d".into(),
            device_name: "d".into(),
            manifests_dir: None,
            backends: Default::default(),
            backups: Default::default(),
        };
        assert!(form.apply_to(&mut cfg).is_err());
    }

    #[test]
    fn sync_summary_formats_outcomes() {
        let p = ResponsePayload::Sync(savesync_core::ipc::SyncPayload {
            results: vec![SyncResult {
                game_key: "elden-ring".into(),
                outcome: "uploaded".into(),
                detail: Some("[fs] uploaded".into()),
            }],
        });
        let s = sync_summary(&p);
        assert!(s.contains("elden-ring"));
        assert!(s.contains("uploaded"));
    }
}
