//! Background update checks/downloads keep IPC polling and egui responsive.
use eframe::egui;
use savesync_core::updates::{self, Release};
use std::sync::mpsc;
enum Event {
    Checked(Result<Option<Release>, String>),
    Installed(Result<(), String>),
}
#[derive(Default)]
pub(super) struct Updates {
    release: Option<Release>,
    job: Option<mpsc::Receiver<Event>>,
    message: String,
    failed: bool,
}
impl Updates {
    fn check(&mut self) {
        self.begin_check(|| updates::check().map_err(|error| error.to_string()));
    }
    fn begin_check(
        &mut self,
        work: impl FnOnce() -> Result<Option<Release>, String> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        self.job = Some(receiver);
        self.message = "Проверка Releases…".into();
        self.failed = false;
        std::thread::spawn(move || {
            let _ = sender.send(Event::Checked(work()));
        });
    }
    pub fn poll(&mut self) {
        let event = match self.job.as_ref().map(|receiver| receiver.try_recv()) {
            Some(Ok(event)) => event,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Event::Installed(Err("Проверка обновления прервана".into()))
            }
            _ => return,
        };
        self.job = None;
        match event {
            Event::Checked(Ok(release)) => {
                self.message = release
                    .as_ref()
                    .map(|release| format!("Доступна версия {}", release.version))
                    .unwrap_or_else(|| "Установлена актуальная версия".into());
                self.release = release;
                self.failed = false;
            }
            Event::Installed(Ok(())) => {
                self.message =
                    "Подписанный установщик запущен; он дождётся завершения синхронизации".into();
                self.failed = false;
            }
            Event::Checked(Err(error)) | Event::Installed(Err(error)) => {
                self.message = error;
                self.failed = true;
            }
        }
    }
    fn install(&mut self) {
        let Some(release) = self.release.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.job = Some(receiver);
        self.message = "Загрузка и проверка подписи…".into();
        std::thread::spawn(move || {
            let _ = sender.send(Event::Installed(
                updates::install(&release).map_err(|error| error.to_string()),
            ));
        });
    }
    pub fn view(&mut self, ui: &mut egui::Ui, busy: bool) {
        ui.heading("Обновления");
        ui.label(format!("SaveSync {}", env!("CARGO_PKG_VERSION")));
        if self.job.is_some() {
            ui.spinner();
        }
        if ui
            .add_enabled(self.job.is_none(), egui::Button::new("Проверить Releases"))
            .clicked()
        {
            self.check();
        }
        if !self.message.is_empty() {
            ui.colored_label(
                if self.failed {
                    egui::Color32::LIGHT_RED
                } else {
                    egui::Color32::LIGHT_BLUE
                },
                &self.message,
            );
        }
        if let Some(release) = self.release.clone() {
            if cfg!(windows) {
                if !updates::automatic_updates_configured() {
                    ui.label("В этой сборке не настроен доверенный издатель обновлений. Установите подписанный релиз вручную.");
                }
                if ui
                    .add_enabled(
                        !busy && self.job.is_none() && updates::automatic_updates_configured(),
                        egui::Button::new(format!("Установить {}", release.version)),
                    )
                    .clicked()
                {
                    self.install();
                }
            } else {
                ui.label("SteamOS: обновите пакет через install-steamos.sh в Desktop Mode; сервис будет перезапущен.");
            }
            if ui.button("Открыть страницу релиза").clicked() {
                let _ = webbrowser::open(&release.page_url);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_failure_is_reported_without_a_desktop() {
        let mut updates = Updates::default();
        updates.begin_check(|| Err("offline".into()));
        let event = updates
            .job
            .as_ref()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(event).unwrap();
        updates.job = Some(receiver);
        updates.poll();
        assert!(updates.failed);
        assert_eq!(updates.message, "offline");
        assert!(updates.job.is_none());
    }
}
