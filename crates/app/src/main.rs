//! SaveSync desktop entry point.
mod application;
mod config_form;
mod daemon_client;
mod presentation;
mod theme;
use eframe::egui;
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
            theme::apply_theme(&cc.egui_ctx);
            Ok(Box::new(application::SaveSyncApp::new()))
        }),
    )
}
