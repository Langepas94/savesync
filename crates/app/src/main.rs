#![cfg_attr(windows, windows_subsystem = "windows")]
//! SaveSync desktop entry point.
mod application;
mod config_form;
mod daemon_client;
mod lifecycle;
mod presentation;
mod theme;
#[cfg(windows)]
mod tray;
use eframe::egui;
fn main() -> eframe::Result<()> {
    if std::env::args().any(|argument| argument == "--version") {
        println!("savesync-app {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|argument| argument == "--quit") {
        if lifecycle::request_exit().is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    let _instance = match lifecycle::instance() {
        Ok(guard) => guard,
        Err(_) => {
            lifecycle::activate_existing();
            return Ok(());
        }
    };
    let startup_error = lifecycle::start_daemon()
        .err()
        .map(|error| error.to_string());
    let minimized = std::env::args().any(|argument| argument == "--tray");
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
            let mut app = application::SaveSyncApp::new();
            app.initialize_desktop(&cc.egui_ctx, minimized, startup_error);
            Ok(Box::new(app))
        }),
    )
}
