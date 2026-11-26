use super::theme::*;
use eframe::egui::{self, RichText};

/// Форматирует время сохранения в UTC.
pub(crate) fn fmt_ts(millis: u64) -> String {
    let secs = millis / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{d:02}.{m:02}.{y:04}, {:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant's civil-from-days (UTC).
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(crate) fn state_badge(ui: &mut egui::Ui, state: &str) {
    let (text, color) = game_state(state);
    ui.label(RichText::new(text).color(color));
}

pub(crate) fn game_state(state: &str) -> (&str, eframe::egui::Color32) {
    match state {
        "ok" => ("Есть синхронизированные версии", OK),
        "conflict" => ("Конфликт версий", WARN),
        "no_versions" => ("Ещё не синхронизировано", DIM),
        "no_saves" => ("Сохранения не найдены", DIM),
        other => (other, DIM),
    }
}
