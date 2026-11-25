use eframe::egui::{self, Color32};
pub(crate) const BG: Color32 = Color32::from_rgb(0x17, 0x1a, 0x21);
pub(crate) const PANEL: Color32 = Color32::from_rgb(0x1b, 0x28, 0x38);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0x66, 0xc0, 0xf4);
pub(crate) const OK: Color32 = Color32::from_rgb(0x7c, 0xc4, 0x6c);
pub(crate) const WARN: Color32 = Color32::from_rgb(0xe5, 0x5a, 0x5a);
pub(crate) const DIM: Color32 = Color32::from_rgb(0x8f, 0x98, 0xa0);

pub(crate) fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x10, 0x13, 0x18);
    v.faint_bg_color = PANEL;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT;
    ctx.set_visuals(v);
}
