//! Состояние desktop, действия пользователя и опрос демона.

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use savesync_core::backends::lan::{DiscoveredPeer, LanPayload};
use savesync_core::config::Config;
use savesync_core::ipc::{HistoryPayload, Method, ResponsePayload, StatusPayload};

use crate::daemon_client::{DaemonClient, IpcDaemonClient};

mod config_form;
mod connections;
mod games;
mod jobs;
mod preferences;
pub(crate) mod updates;

use config_form::ConfigForm;
pub(crate) use games::AddGameForm;
use jobs::Job;

#[cfg(test)]
#[path = "../tests/application.rs"]
mod tests;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Tab {
    Games,
    Connections,
    Cloud,
    Backups,
    Settings,
}

pub(crate) struct Application {
    oauth_poll: Option<
        mpsc::Receiver<(
            savesync_core::auth::oauth::Provider,
            Result<ResponsePayload, String>,
        )>,
    >,
    pub(crate) updates: updates::Updates,
    pub(crate) lan: Option<LanPayload>,
    lan_poll: Option<mpsc::Receiver<Result<ResponsePayload, String>>>,
    lan_presence_poll: Option<mpsc::Receiver<Result<ResponsePayload, String>>>,
    pub(crate) online_peers: Option<Vec<String>>,
    last_presence_poll: Instant,
    pub(crate) lan_discovered: Option<Vec<DiscoveredPeer>>,
    pub(crate) lan_address: String,
    pub(crate) lan_pair_pin: String,
    pub(crate) scroll_to_lan_results: bool,
    pub(crate) oauth_pending: Option<savesync_core::auth::oauth::Provider>,
    pub(crate) yandex_client_id: String,
    pub(crate) yandex_notice: Option<(String, bool)>,
    pub(crate) yandex_check: Option<mpsc::Receiver<Result<ResponsePayload, String>>>,
    client: Arc<dyn DaemonClient>,
    pub(crate) tab: Tab,
    pub(crate) add_game: Option<AddGameForm>,
    pub(crate) daemon_ok: bool,
    pub(crate) status: Option<StatusPayload>,
    pub(crate) notice: Option<(String, bool)>, // (текст, это ошибка)
    pub(crate) selected: Option<String>,
    pub(crate) history: Option<HistoryPayload>,
    pub(crate) history_limit: usize,
    pub(crate) restore_confirmation: Option<(String, String, String)>,
    pub(crate) backup_picker: Option<mpsc::Receiver<Option<std::path::PathBuf>>>,
    pub(crate) cfg: Option<Config>,
    pub(crate) form: ConfigForm,
    form_loaded: bool,
    job: Option<Job>,
    last_poll: Instant,
    last_probe: Instant,
}

impl Application {
    pub(crate) fn new() -> Self {
        Application {
            oauth_poll: None,
            updates: updates::Updates::default(),
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

    pub(crate) fn set_notice(&mut self, text: String, is_error: bool) {
        self.notice = Some((text, is_error));
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

    pub(crate) fn busy(&self) -> bool {
        self.job.is_some() || self.status.as_ref().is_some_and(|status| status.busy)
    }
    pub(crate) fn tick(&mut self) {
        self.collect_oauth_poll();
        self.collect_yandex_check();
        self.updates.poll();

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

        self.finish_job();
    }

    pub(crate) fn probe_daemon(&mut self) {
        if self.last_probe.elapsed() >= Duration::from_secs(3) {
            self.poll_status();
            self.last_probe = Instant::now();
        }
    }
}
