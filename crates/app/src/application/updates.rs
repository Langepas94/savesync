//! Background update checks/downloads keep IPC polling and egui responsive.
use savesync_core::updates::{self, Release, ReleaseCheck};
use std::sync::mpsc;
enum Event {
    Checked(Result<ReleaseCheck, String>),
    Installed(Result<(), String>),
}
#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) release: Option<Release>,
    job: Option<mpsc::Receiver<Event>>,
    pub(crate) message: String,
    pub(crate) failed: bool,
    last_check: Option<std::time::Instant>,
}
impl Updates {
    pub(crate) fn check(&mut self) {
        self.last_check = Some(std::time::Instant::now());
        self.begin_check(|| updates::check().map_err(|error| error.to_string()));
    }
    fn begin_check(
        &mut self,
        work: impl FnOnce() -> Result<ReleaseCheck, String> + Send + 'static,
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
        if self.job.is_none()
            && self
                .last_check
                .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(86400))
        {
            self.check();
        }
        let event = match self.job.as_ref().map(|receiver| receiver.try_recv()) {
            Some(Ok(event)) => event,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Event::Installed(Err("Проверка обновления прервана".into()))
            }
            _ => return,
        };
        self.job = None;
        match event {
            Event::Checked(Ok(result)) => {
                self.release = None;
                self.message = match result {
                    ReleaseCheck::NoPublishedRelease => {
                        "В GitHub Releases пока нет стабильных релизов SaveSync.".into()
                    }
                    ReleaseCheck::UpToDate => "Установлена актуальная версия".into(),
                    ReleaseCheck::Available(release) => {
                        let message = format!("Доступна версия {}", release.version);
                        self.release = Some(release);
                        message
                    }
                };
                self.failed = false;
            }
            Event::Installed(Ok(())) => {
                self.message =
                    "Обновление запущено; программа дождётся завершения синхронизации".into();
                self.failed = false;
            }
            Event::Checked(Err(error)) => {
                self.release = None;
                self.message = error;
                self.failed = true;
            }
            Event::Installed(Err(error)) => {
                self.message = error;
                self.failed = true;
            }
        }
    }
    pub(crate) fn install(&mut self) {
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
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }
}
#[cfg(test)]
#[path = "../tests/updates.rs"]
mod tests;
