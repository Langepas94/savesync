use super::theme::DIM;
use crate::application::Application;
use eframe::egui::{self, RichText};

impl Application {
    pub(crate) fn view_settings(&mut self, ui: &mut egui::Ui) {
        let busy = self.busy();
        ui.heading("Настройки");
        ui.add_space(8.0);
        super::updates::show(&mut self.updates, ui, busy);
        ui.separator();
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
    }
}
