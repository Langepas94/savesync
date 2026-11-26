use super::{presentation::*, theme::*};
use crate::application::Application;
use eframe::egui::{self, RichText};
use std::sync::mpsc;

impl Application {
    pub(crate) fn view_backups(&mut self, ui: &mut egui::Ui) {
        let busy = self.busy();
        ui.heading("Бэкапы");
        let Some(config) = &self.cfg else {
            ui.label("Загрузка настроек…");
            return;
        };
        if let Some(hours) = config.backups.auto_interval_hours {
            ui.colored_label(
                OK,
                format!("Автоматические бэкапы включены · каждые {hours} ч"),
            );
        } else {
            ui.label("Автоматические бэкапы выключены");
        }
        if let Some(directory) = &config.backups.dir {
            ui.label("Папка хранения");
            ui.add(
                egui::Label::new(RichText::new(directory.display().to_string()).monospace()).wrap(),
            );
        }
        if let Some(last) = self
            .status
            .as_ref()
            .and_then(|status| status.last_backup.as_ref())
        {
            ui.label(format!("Последний бэкап · {}", fmt_ts(last.created_at)));
            ui.add(egui::Label::new(RichText::new(&last.directory).monospace()).wrap());
        }
        if ui
            .add_enabled(
                !busy && config.backups.dir.is_some(),
                primary_button("Создать сейчас"),
            )
            .clicked()
        {
            self.backup_all();
        }
        ui.separator();
        let result = self.backup_picker.as_ref().map(mpsc::Receiver::try_recv);
        match result {
            Some(Ok(path)) => {
                self.backup_picker = None;
                if let Some(path) = path {
                    self.form.backups_dir = path.display().to_string();
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.backup_picker = None;
                self.set_notice("Не удалось открыть выбор папки".into(), true);
            }
            _ => (),
        }
        ui.add_enabled_ui(!busy, |ui| {
            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Бэкапы").strong());
                ui.horizontal(|ui| {
                    ui.label("Изменить папку хранения");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.backups_dir).desired_width(340.0),
                    );
                    if ui
                        .add_enabled(
                            self.backup_picker.is_none(),
                            egui::Button::new("Выбрать папку…"),
                        )
                        .clicked()
                    {
                        let (sender, receiver) = mpsc::channel();
                        let ctx = ui.ctx().clone();
                        self.backup_picker = Some(receiver);
                        std::thread::spawn(move || {
                            let selected = rfd::FileDialog::new()
                                .set_title("Папка бэкапов SaveSync")
                                .pick_folder();
                            let _ = sender.send(selected);
                            ctx.request_repaint();
                        });
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Хранить копий на игру:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.backups_keep).desired_width(60.0),
                    );
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
                self.save_backups();
            }
        });
    }
}
