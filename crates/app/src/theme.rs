use eframe::egui::{self, Color32};
pub(crate) const BG: Color32 = Color32::from_rgb(0x11, 0x17, 0x20);
pub(crate) const PANEL: Color32 = Color32::from_rgb(0x1a, 0x25, 0x33);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0xa9, 0xc9, 0xff);
pub(crate) const OK: Color32 = Color32::from_rgb(0x80, 0xe6, 0xb2);
pub(crate) const WARN: Color32 = Color32::from_rgb(0xff, 0xb9, 0x91);
pub(crate) const DIM: Color32 = Color32::from_rgb(0xa6, 0xb7, 0xcd);

pub(crate) fn primary_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        egui::RichText::new(label)
            .strong()
            .color(Color32::from_rgb(0x12, 0x28, 0x4a)),
    )
    .fill(ACCENT)
    .rounding(6.0)
}

pub(crate) fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x10, 0x13, 0x18);
    v.faint_bg_color = PANEL;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(0x24, 0x3b, 0x59);
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    v.widgets.inactive.weak_bg_fill = PANEL;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x24, 0x3b, 0x59);
    ctx.set_visuals(v);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}
