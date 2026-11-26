use super::*;
use savesync_core::ipc::{LocalSaveFile, VersionEntry};
use std::{collections::BTreeMap, path::Path};

impl SaveSyncApp {
    pub(super) fn view_detail(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        if ui.button("← К списку игр").clicked() {
            self.selected = None;
            self.history = None;
            self.restore_confirmation = None;
            return;
        }
        let game = self
            .status
            .as_ref()
            .and_then(|s| s.games.iter().find(|g| g.game_key == key));
        let name = game.map_or(key.as_str(), |g| &g.display_name);
        ui.heading(RichText::new(name).size(27.0));
        if let Some(game) = game {
            state_badge(ui, &game.state);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Синхронизировать"))
                .clicked()
            {
                self.sync_game(&key);
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Сделать резервную копию"))
                .clicked()
            {
                self.backup(&key);
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Убрать из списка"))
                .clicked()
            {
                self.set_game_hidden(key.clone(), true);
                self.selected = None;
                self.history = None;
            }
        });
        if self.selected.is_none() {
            return;
        }
        ui.separator();
        let Some(history) = self.history.as_ref().filter(|h| h.game_key == key) else {
            if self.job.is_some() {
                ui.label("Загрузка истории…");
            } else if ui.button("Загрузить историю снова").clicked() {
                self.load_history(&key);
            }
            return;
        };
        let busy = self.busy();
        let current = history
            .versions
            .iter()
            .find(|v| history.applied.as_deref() == Some(&v.id));
        let conflict = history.heads.len() > 1;
        let local_id = self
            .status
            .as_ref()
            .map(|s| s.device_id.as_str())
            .or_else(|| self.cfg.as_ref().map(|c| c.device_id.as_str()));
        let mut choice = None;
        if conflict {
            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.colored_label(WARN, "Прогресс изменился на нескольких устройствах");
                ui.label("Выберите версию целиком. Все исходные версии сохранятся в истории.");
                for version in history
                    .versions
                    .iter()
                    .rev()
                    .filter(|v| history.heads.contains(&v.id))
                {
                    if version_row(
                        ui,
                        version,
                        local_id,
                        current,
                        Some("Выбрать эту копию"),
                        busy,
                    ) {
                        choice = Some(version);
                    }
                }
            });
            ui.add_space(9.0);
        }
        show_local_paths(ui, &history.install_paths, &history.local_files);
        ui.add_space(10.0);
        ui.heading("История версий SaveSync");
        if history.versions.is_empty() {
            ui.label("Первая версия появится после синхронизации сохранений.");
        } else {
            ui.small("Каждая версия содержит набор файлов сохранений на момент копирования.");
            let mut shown = 0;
            for version in history.versions.iter().rev() {
                let is_current = current.is_some_and(|v| v.id == version.id);
                if shown >= self.history_limit && !is_current {
                    continue;
                }
                shown += 1;
                let same_content =
                    current.is_some_and(|v| !v.hash.is_empty() && v.hash == version.hash);
                let label = (!conflict && !is_current && !same_content).then_some("Восстановить");
                if version_row(ui, version, local_id, current, label, busy) {
                    choice = Some(version);
                }
                if is_current && !conflict {
                    ui.push_id(&version.id, |ui| {
                        ui.collapsing("Дополнительные действия", |ui| {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Восстановить файлы заново"))
                                .clicked()
                            {
                                choice = Some(version);
                            }
                        });
                    });
                }
            }
            if shown < history.versions.len() && ui.button("Показать более ранние версии").clicked()
            {
                self.history_limit += 20;
            }
        }
        if let Some(version) = choice {
            let mut description = format!(
                "{} · {}",
                fmt_ts(version.created_at),
                version_device(version, local_id)
            );
            if let Some(count) = version.file_count {
                description.push_str(&format!(" · файлов в версии: {count}"));
            }
            self.restore_confirmation = Some((key.clone(), version.id.clone(), description));
        }
        self.show_restore_confirmation(ui, &key);
    }

    fn show_restore_confirmation(&mut self, ui: &mut egui::Ui, selected: &str) {
        let Some((game, version, description)) = self.restore_confirmation.clone() else {
            return;
        };
        if game != selected {
            self.restore_confirmation = None;
            return;
        }
        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Применить выбранную версию?")
            .id(egui::Id::new("confirm_restore"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(390.0)
            .show(ui.ctx(), |ui| {
                ui.label(description);
                ui.label("SaveSync восстановит весь набор файлов этой версии и сохранит резервную копию перед заменой. Закройте игру перед продолжением.");
                ui.horizontal_wrapped(|ui| {
                    confirmed = ui.add_enabled(!self.busy(), primary_button("Подтвердить восстановление")).clicked();
                    cancelled = ui.button("Отмена").clicked();
                });
            });
        if !open || cancelled || confirmed {
            self.restore_confirmation = None;
        }
        if confirmed {
            self.restore(&game, &version);
        }
    }
}

fn show_local_paths(ui: &mut egui::Ui, install: &[String], files: &[LocalSaveFile]) {
    let mut folders = BTreeMap::<&Path, Vec<&LocalSaveFile>>::new();
    for file in files {
        let path = Path::new(&file.path);
        folders
            .entry(path.parent().unwrap_or(path))
            .or_default()
            .push(file);
    }
    egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
        ui.heading("Пути на этом устройстве");
        ui.label(RichText::new("Папка установки").strong());
        if install.is_empty() {
            ui.colored_label(DIM, "Не определена");
        }
        for path in install {
            ui.add(egui::Label::new(RichText::new(path).monospace()).wrap());
        }
        ui.label(RichText::new("Сохранения").strong());
        if folders.is_empty() {
            ui.colored_label(DIM, "Сохранения не найдены");
        }
        for (folder, files) in folders {
            let path = folder.to_string_lossy();
            ui.push_id(folder, |ui| {
                ui.add(egui::Label::new(RichText::new(path.as_ref()).monospace()).wrap());
                let label = known_profile(folder).map_or_else(
                    || format!("Файлы в папке ({})", files.len()),
                    |profile| format!("Профиль {profile} · файлов: {}", files.len()),
                );
                ui.collapsing(label, |ui| {
                    for file in files {
                        let name = Path::new(&file.path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        ui.add(egui::Label::new(RichText::new(name).monospace()).wrap())
                            .on_hover_text(&file.path);
                        ui.small(format!(
                            "{} байт · {}",
                            file.size,
                            file.modified_at
                                .map_or_else(|| "Время неизвестно".into(), fmt_ts)
                        ));
                    }
                });
            });
        }
    });
}

fn known_profile(folder: &Path) -> Option<&str> {
    let name = folder.file_name()?.to_str()?;
    let profiles = folder.parent()?;
    let title = profiles.parent()?.file_name()?.to_str()?;
    if profiles
        .file_name()?
        .to_str()?
        .eq_ignore_ascii_case("Profiles")
        && title.eq_ignore_ascii_case("Red Dead Redemption 2")
        && name.len() == 8
        && name.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Some(name)
    } else {
        None
    }
}

fn version_device(version: &VersionEntry, local_id: Option<&str>) -> String {
    if local_id == Some(&version.device) {
        return "Создана на этом устройстве".into();
    }
    let fallback = version
        .device
        .rsplit_once('-')
        .filter(|(name, suffix)| {
            !name.is_empty() && suffix.len() == 8 && suffix.bytes().all(|c| c.is_ascii_hexdigit())
        })
        .map_or(version.device.as_str(), |(name, _)| name);
    let name = version
        .device_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(fallback);
    match name {
        "" => "Источник неизвестен".into(),
        "steamdeck" => "Steam Deck".into(),
        _ => name.into(),
    }
}

fn version_row(
    ui: &mut egui::Ui,
    version: &VersionEntry,
    local_id: Option<&str>,
    current: Option<&VersionEntry>,
    action: Option<&str>,
    busy: bool,
) -> bool {
    let mut clicked = false;
    egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(version_device(version, local_id)).strong());
            ui.label(fmt_ts(version.created_at));
            if let Some(channel) = version.channel.as_deref() {
                ui.small(match channel {
                    "lan" => "Локальная сеть",
                    "yandex-disk" => "Яндекс Диск",
                    "google-drive" => "Google Диск",
                    "fs" => "Локальная папка / USB",
                    other => other,
                });
            }
            if let Some(count) = version.file_count {
                ui.small(format!("Файлов: {count}"));
            }
            if current.is_some_and(|v| v.id == version.id) {
                ui.colored_label(OK, "На этом устройстве").on_hover_text(
                    "Последняя версия, которую SaveSync применил или записал здесь. Игра могла изменить файлы после этого.",
                );
            } else if current.is_some_and(|v| !v.hash.is_empty() && v.hash == version.hash) {
                ui.small("Содержимое уже на устройстве");
            }
            if let Some(action) = action {
                clicked = ui.add_enabled(!busy, egui::Button::new(action)).clicked();
            }
        });
    });
    clicked
}
