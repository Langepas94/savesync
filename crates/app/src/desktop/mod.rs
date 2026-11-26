//! Запуск desktop и жизненный цикл окна.

use crate::application::Application;

mod lifecycle;
#[cfg(windows)]
mod tray;

use eframe::egui;
use std::sync::Arc;

pub(crate) fn run() -> eframe::Result<()> {
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
    let icon = Arc::new(
        eframe::icon_data::from_png_bytes(include_bytes!("../../assets/savesync.png"))
            .expect("bundled SaveSync icon must be valid"),
    );
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(Arc::clone(&icon))
            .with_inner_size([980.0, 660.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SaveSync",
        options,
        Box::new(move |cc| {
            crate::ui::theme::apply_theme(&cc.egui_ctx);
            let mut app = Desktop::new();
            app.initialize_desktop(&cc.egui_ctx, minimized, startup_error, &icon);
            Ok(Box::new(app))
        }),
    )
}

struct Desktop {
    application: Application,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    #[cfg(windows)]
    allow_exit: bool,
}

impl Desktop {
    fn new() -> Self {
        Self {
            application: Application::new(),
            #[cfg(windows)]
            tray: None,
            #[cfg(windows)]
            allow_exit: false,
        }
    }

    fn initialize_desktop(
        &mut self,
        context: &egui::Context,
        minimized: bool,
        startup_error: Option<String>,
        icon: &egui::IconData,
    ) {
        #[cfg(windows)]
        {
            match tray::Tray::new(context, icon) {
                Ok(tray) => {
                    self.tray = Some(tray);
                    if minimized {
                        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    }
                }
                Err(error) => self
                    .application
                    .set_notice(format!("Трей недоступен: {error}"), true),
            }
        }
        #[cfg(not(windows))]
        let _ = (context, minimized, icon);
        if let Some(error) = startup_error {
            self.application.set_notice(error, true);
        }
    }
    #[cfg(windows)]
    fn poll_tray(&mut self, context: &egui::Context) {
        while let Some(action) = self.tray.as_ref().and_then(|tray| tray.next()) {
            match action {
                tray::Action::Open => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::Action::Hide => {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(false))
                }
                tray::Action::Sync => {
                    if !self.application.busy() {
                        self.application.sync_all();
                    }
                }
                tray::Action::Quit => {
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
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if lifecycle::consume_exit_request() {
            #[cfg(windows)]
            {
                self.allow_exit = true;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(windows)]
        self.poll_tray(ctx);
        self.application.tick();
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        self.application.show(ctx, lifecycle::start_daemon);
    }
}
