use super::games::sync_summary;
use super::*;
use crate::ui::presentation::*;
use eframe::egui;
use savesync_core::config::{LanConfig, YandexConfig};
use savesync_core::ipc::SyncResult;

#[path = "application_games.rs"]
mod games;
#[path = "application_lan.rs"]
mod lan;
#[path = "application_oauth.rs"]
mod oauth;
#[path = "application_preferences.rs"]
mod preferences;
#[path = "application_presentation.rs"]
mod presentation;
