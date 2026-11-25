use super::*;
impl SaveSyncApp {
    pub(super) fn view_games(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Игры");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Синхронизировать всё"))
                    .clicked()
                {
                    self.sync_all();
                }
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Обновить"))
                    .clicked()
                {
                    self.refresh_games();
                }
            });
        });
        ui.separator();

        let games = self
            .status
            .as_ref()
            .map(|s| s.games.clone())
            .unwrap_or_default();
        if games.is_empty() {
            ui.label(RichText::new("Игры не найдены. Нажмите «Обновить» для поиска.").color(DIM));
        }

        let mut sync_one = None;
        let mut select_one = None;
        egui::ScrollArea::both()
            .id_salt("games")
            .max_height(ui.available_height() * 0.45)
            .show(ui, |ui| {
                for g in &games {
                    let is_sel = self.selected.as_deref() == Some(g.game_key.as_str());
                    let frame = egui::Frame::group(ui.style()).fill(if is_sel {
                        Color32::from_rgb(0x24, 0x35, 0x47)
                    } else {
                        PANEL
                    });
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&g.display_name).strong());
                            ui.label(
                                RichText::new(format!("версий: {}", g.versions))
                                    .color(DIM)
                                    .small(),
                            );
                            state_badge(ui, &g.state);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(!self.busy(), egui::Button::new("Синк"))
                                        .clicked()
                                    {
                                        sync_one = Some(g.game_key.clone());
                                    }
                                    if ui
                                        .add_enabled(
                                            !self.busy(),
                                            egui::Button::new(if is_sel {
                                                "Скрыть"
                                            } else {
                                                "Детали"
                                            }),
                                        )
                                        .clicked()
                                    {
                                        select_one = Some(if is_sel {
                                            None
                                        } else {
                                            Some(g.game_key.clone())
                                        });
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(2.0);
                }
            });

        if let Some(key) = sync_one {
            self.sync_game(&key);
        }
        if let Some(sel) = select_one {
            match sel {
                Some(key) => {
                    self.selected = Some(key.clone());
                    self.load_history(&key);
                }
                None => {
                    self.selected = None;
                    self.history = None;
                }
            }
        }

        ui.separator();
        self.view_detail(ui);
    }

    pub(super) fn view_detail(&mut self, ui: &mut egui::Ui) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        ui.heading("Детали");
        ui.horizontal(|ui| {
            ui.label(RichText::new(&key).color(ACCENT));
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Сделать бэкап"))
                .clicked()
            {
                self.backup(&key);
            }
        });

        let Some(h) = self.history.clone() else {
            ui.label(RichText::new("нет истории").color(DIM));
            return;
        };
        if h.game_key != key {
            return;
        }

        // Конфликт-резолвер: больше одной головы.
        if h.heads.len() > 1 {
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(0x3a, 0x24, 0x24))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Конфликт: {} ветки. Выберите версию ниже — она станет общей.",
                            h.heads.len()
                        ))
                        .color(WARN),
                    );
                });
        }

        let mut restore_id = None;
        egui::ScrollArea::vertical()
            .id_salt("history")
            .show(ui, |ui| {
                for v in h.versions.iter().rev() {
                    egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let is_applied = h.applied.as_deref() == Some(v.id.as_str());
                            let is_head = h.heads.contains(&v.id);
                            ui.label(format!("{} UTC", fmt_ts(v.created_at)));
                            ui.label(RichText::new(&v.device).color(DIM));
                            ui.label(
                                RichText::new(&v.id[..v.id.len().min(12)])
                                    .color(DIM)
                                    .monospace()
                                    .small(),
                            );
                            if is_head {
                                ui.label(RichText::new("голова").color(WARN).small());
                            }
                            if is_applied {
                                ui.label(RichText::new("применена").color(OK).small());
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(!self.busy(), egui::Button::new("Применить"))
                                        .clicked()
                                    {
                                        restore_id = Some(v.id.clone());
                                    }
                                },
                            );
                        });
                    });
                    ui.add_space(2.0);
                }
            });
        if let Some(id) = restore_id {
            self.restore(&key, &id);
        }
    }
}
