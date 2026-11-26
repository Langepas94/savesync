#![cfg_attr(windows, windows_subsystem = "windows")]
//! SaveSync desktop entry point.
mod application;
mod daemon_client;
mod desktop;
mod ui;

fn main() -> eframe::Result<()> {
    desktop::run()
}
