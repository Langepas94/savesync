use super::*;

#[derive(Default)]
pub(super) struct AddGameForm {
    name: String,
    paths: String,
    pub(super) error: Option<String>,
    picker: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
}

impl SaveSyncApp {
    pub(super) fn view_games(&mut self, ui: &mut egui::Ui) {
        if let Some(selected) = &self.selected {
            egui::ScrollArea::vertical()
                .id_salt(("game_detail", selected))
                .show(ui, |ui| {
                    self.view_detail(ui);
                });
            return;
        }

        ui.heading(RichText::new("Игры").size(27.0));
        let mut opened_form = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.daemon_ok && !self.busy() && self.add_game.is_none(),
                    egui::Button::new("Добавить игру"),
                )
                .clicked()
            {
                self.add_game = Some(AddGameForm::default());
                opened_form = true;
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Обновить список"))
                .clicked()
            {
                self.refresh_games();
            }
            if ui
                .add_enabled(!self.busy(), primary_button("Синхронизировать всё"))
                .clicked()
            {
                self.sync_all();
            }
        });
        ui.separator();

        let games = self
            .status
            .as_ref()
            .map(|s| s.games.clone())
            .unwrap_or_default();
        let hidden = self
            .status
            .as_ref()
            .map(|s| s.hidden_games.clone())
            .unwrap_or_default();
        let mut list = egui::ScrollArea::vertical()
            .id_salt("library")
            .auto_shrink([false, false]);
        if opened_form {
            list = list.vertical_scroll_offset(0.0);
        }
        list.show(ui, |ui| {
            self.view_add_game(ui);
            if games.is_empty() {
                ui.label("Обновите список или добавьте игру вручную.");
            }
            for game in &games {
                ui.push_id(&game.game_key, |ui| {
                    let (state, color) = game_state(&game.state);
                    let mut label = egui::text::LayoutJob::default();
                    label.append(
                        &game.display_name,
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(16.0),
                            color: ui.visuals().text_color(),
                            ..Default::default()
                        },
                    );
                    label.append(
                        &format!("   •   {state}"),
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(13.0),
                            color,
                            ..Default::default()
                        },
                    );
                    if ui
                        .add_enabled(
                            !self.busy(),
                            egui::Button::new(label)
                                .min_size(egui::vec2(ui.available_width(), 39.0)),
                        )
                        .clicked()
                    {
                        self.selected = Some(game.game_key.clone());
                        self.history_limit = 8;
                        self.load_history(&game.game_key);
                    }
                });
            }
            if !hidden.is_empty() {
                ui.collapsing("Удалённые из списка", |ui| {
                    for game in hidden {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(&game.display_name);
                            if ui
                                .add_enabled(!self.busy(), egui::Button::new("Вернуть"))
                                .clicked()
                            {
                                self.set_game_hidden(game.game_key, false);
                            }
                        });
                    }
                });
            }
        });
    }

    pub(super) fn set_game_hidden(&mut self, game_key: String, hidden: bool) {
        self.start_job(
            "Обновление списка игр…",
            Method::SetGameHidden { game_key, hidden },
            move |payload| match payload {
                ResponsePayload::Scan(_) => (
                    if hidden {
                        "Игра удалена из списка"
                    } else {
                        "Игра возвращена в список"
                    }
                    .into(),
                    true,
                    false,
                ),
                ResponsePayload::Sync(result) => (
                    format!(
                        "Ошибка: {}",
                        result
                            .results
                            .first()
                            .and_then(|r| r.detail.as_deref())
                            .unwrap_or("не удалось изменить список")
                    ),
                    true,
                    false,
                ),
                _ => ("Неожиданный ответ демона".into(), true, false),
            },
        );
    }

    fn view_add_game(&mut self, ui: &mut egui::Ui) {
        let can_save = self.daemon_ok && !self.busy();
        let submitting = self.job.as_ref().is_some_and(|job| job.adding_game);
        let Some(form) = &mut self.add_game else {
            return;
        };
        let mut submitted = false;
        let mut cancelled = false;
        if let Some(picker) = &form.picker {
            match picker.try_recv() {
                Ok(path) => {
                    if let Some(path) = path {
                        if !form.paths.trim().is_empty() {
                            form.paths.push('\n');
                        }
                        form.paths.push_str(&path.to_string_lossy());
                    }
                    form.picker = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    form.error = Some("Не удалось открыть выбор папки".into());
                    form.picker = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            if submitting {
                ui.disable();
            }
            ui.label("Название игры");
            ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(420.0));
            ui.label("Папка игры или файлы сохранений");
            if ui
                .add_enabled(form.picker.is_none(), egui::Button::new("Выбрать папку…"))
                .clicked()
            {
                let (sender, receiver) = std::sync::mpsc::channel();
                let context = ui.ctx().clone();
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .set_title("Выберите папку игры или сохранений")
                        .pick_folder();
                    let _ = sender.send(path);
                    context.request_repaint();
                });
                form.picker = Some(receiver);
            }
            ui.add(
                egui::TextEdit::multiline(&mut form.paths)
                    .desired_width(420.0)
                    .desired_rows(2),
            );
            ui.small("Вставьте полный путь к каждому файлу или папке с новой строки.");
            ui.small("Папка игры определяется по названию и доступной базе игр. Если игра не найдена, укажите папку сохранений.");
            if let Some(error) = &form.error {
                ui.colored_label(WARN, error);
            }
            ui.horizontal(|ui| {
                let complete = !form.name.trim().is_empty() && !form.paths.trim().is_empty();
                submitted = ui
                    .add_enabled(can_save && complete, egui::Button::new("Добавить"))
                    .clicked();
                cancelled = ui.button("Отмена").clicked();
            });
        });
        if cancelled {
            self.add_game = None;
        } else if submitted {
            let name = form.name.trim().to_string();
            let game_key = savesync_core::game_key::custom_key(&name);
            let display_name = name.clone();
            let paths = form
                .paths
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| {
                    line.strip_prefix('"')
                        .and_then(|p| p.strip_suffix('"'))
                        .unwrap_or(line)
                        .to_string()
                })
                .collect();
            form.error = None;
            self.start_job("Добавление игры…", Method::AddCustomGame { name, paths }, move |payload| {
                let no_saves = matches!(payload, ResponsePayload::Scan(scan)
                    if scan.games.iter().any(|game| (game.game_key == game_key || game.display_name == display_name) && game.locations == 0));
                let message = if no_saves { "Игра добавлена. Сохранения не найдены." } else { "Игра добавлена" };
                (message.into(), true, false)
            });
        }
    }
}
