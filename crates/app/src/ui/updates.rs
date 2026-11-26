use eframe::egui;
use savesync_core::updates;

use crate::application::updates::Updates;

pub(super) fn show(state: &mut Updates, ui: &mut egui::Ui, busy: bool) {
    ui.heading("Обновления");
    ui.label(format!("SaveSync {}", env!("CARGO_PKG_VERSION")));
    ui.small("Автоматическая проверка при запуске и каждые 24 часа работы приложения.");
    if state.busy() {
        ui.spinner();
    }
    if ui
        .add_enabled(!state.busy(), egui::Button::new("Проверить обновления"))
        .clicked()
    {
        state.check();
    }
    if !state.message.is_empty() {
        ui.colored_label(
            if state.failed {
                egui::Color32::LIGHT_RED
            } else {
                egui::Color32::LIGHT_BLUE
            },
            &state.message,
        );
    }
    if let Some(release) = state.release.clone() {
        if cfg!(any(windows, target_os = "linux")) {
            if !updates::automatic_updates_configured() {
                ui.label("В этой сборке не настроен доверенный ключ обновлений.");
            }
            if ui
                .add_enabled(
                    !busy && !state.busy() && updates::automatic_updates_configured(),
                    egui::Button::new(format!("Установить {}", release.version)),
                )
                .clicked()
            {
                state.install();
            }
        } else {
            ui.label("Автоматическое обновление на этой платформе недоступно.");
        }
        if ui.button("Открыть страницу релиза").clicked() {
            let _ = webbrowser::open(&release.page_url);
        }
    }
}
