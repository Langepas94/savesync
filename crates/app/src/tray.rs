//! Windows tray, created on eframe's UI thread. Tests exercise dispatch without an OS tray.
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Open,
    Sync,
    Hide,
    Quit,
}
pub(crate) struct Tray {
    _icon: TrayIcon,
    events: Receiver<Action>,
}
impl Tray {
    pub fn new(context: &egui::Context) -> Result<Self, String> {
        let menu = Menu::new();
        let open = MenuItem::new("Открыть SaveSync", true, None);
        let sync = MenuItem::new("Синхронизировать", true, None);
        let hide = MenuItem::new("Скрыть окно", true, None);
        let quit = MenuItem::new("Выход из приложения", true, None);
        menu.append_items(&[&open, &sync, &hide, &quit])
            .map_err(|error| error.to_string())?;
        let ids = [
            (open.id().clone(), Action::Open),
            (sync.id().clone(), Action::Sync),
            (hide.id().clone(), Action::Hide),
            (quit.id().clone(), Action::Quit),
        ];
        let (sender, events) = mpsc::channel();
        let context = context.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(action) = dispatch(&ids, &event.id) {
                let _ = sender.send(action);
                context.request_repaint();
            }
        }));
        let mut pixels = vec![0; 32 * 32 * 4];
        for y in 0..32 {
            for x in 0..32 {
                let offset = (y * 32 + x) * 4;
                let inside = (x as i32 - 16).pow(2) + (y as i32 - 16).pow(2) < 225;
                let stripe = (8..24).contains(&x)
                    && ((7..11).contains(&y) || (14..18).contains(&y) || (21..25).contains(&y))
                    || ((8..12).contains(&x) && (9..16).contains(&y))
                    || ((20..24).contains(&x) && (16..23).contains(&y));
                pixels[offset..offset + 4].copy_from_slice(if inside {
                    if stripe {
                        &[23, 26, 33, 255]
                    } else {
                        &[102, 192, 244, 255]
                    }
                } else {
                    &[0, 0, 0, 0]
                });
            }
        }
        let icon = Icon::from_rgba(pixels, 32, 32).map_err(|error| error.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("SaveSync")
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _icon: icon,
            events,
        })
    }
    pub fn next(&self) -> Option<Action> {
        self.events.try_recv().ok()
    }
}
fn dispatch(ids: &[(MenuId, Action)], id: &MenuId) -> Option<Action> {
    ids.iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, action)| *action)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_dispatches_only_known_actions() {
        let id = MenuId::new("sync");
        let ids = [(id.clone(), Action::Sync)];
        assert_eq!(dispatch(&ids, &id), Some(Action::Sync));
        assert_eq!(dispatch(&ids, &MenuId::new("other")), None);
    }
}
