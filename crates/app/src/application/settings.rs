use super::*;
impl SaveSyncApp {
    pub(super) fn view_connections(&mut self, ui: &mut egui::Ui) {
        ui.heading("Подключения");
        ui.label(
            RichText::new(
                "Каналы независимы. Подключите облако через браузер или укажите готовый токен.",
            )
            .color(DIM)
            .small(),
        );
        ui.add_space(8.0);

        if let Some(provider) = self.oauth_pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Ожидание входа в браузере");
                if ui
                    .add_enabled(!self.busy(), egui::Button::new("Отменить вход"))
                    .clicked()
                {
                    self.cancel_oauth(provider);
                }
            });
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Локальная папка / USB (fs)").strong());
                ui.horizontal(|ui| {
                    ui.label("Путь:");
                    ui.add(egui::TextEdit::singleline(&mut self.form.fs_path).desired_width(420.0));
                });
            });
            ui.add_space(6.0);

            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Яндекс.Диск").strong());
                if ui
                    .add_enabled(
                        !self.busy() && self.oauth_pending.is_none(),
                        egui::Button::new("Подключить Яндекс через браузер"),
                    )
                    .clicked()
                {
                    self.begin_oauth(savesync_core::oauth::Provider::Yandex);
                }
                ui.horizontal(|ui| {
                    ui.label("OAuth-токен:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.yandex_token)
                            .password(true)
                            .desired_width(380.0),
                    );
                });
            });
            ui.add_space(6.0);

            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("Google Drive").strong());
                if ui
                    .add_enabled(
                        !self.busy() && self.oauth_pending.is_none(),
                        egui::Button::new("Подключить Google через браузер"),
                    )
                    .clicked()
                {
                    self.begin_oauth(savesync_core::oauth::Provider::Google);
                }
                ui.horizontal(|ui| {
                    ui.label("Access token:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.google_token)
                            .password(true)
                            .desired_width(380.0),
                    );
                });
            });
            ui.add_space(6.0);

            egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
                ui.label(RichText::new("LAN (прямая синхронизация по сети)").strong());
                ui.checkbox(&mut self.form.lan_enabled, "включить LAN-канал");
                if self.form.lan_enabled {
                    ui.horizontal(|ui| {
                        ui.label("Порт:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_port).desired_width(80.0),
                        );
                        ui.label("PIN:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_pin)
                                .password(true)
                                .desired_width(120.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Пиры (host:port, через запятую):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_peers)
                                .desired_width(320.0),
                        );
                    });
                }
            });
            ui.add_space(10.0);

            if ui
                .add_enabled(!self.busy(), egui::Button::new("Сохранить"))
                .clicked()
            {
                self.save_config();
            }
        });
    }

    pub(super) fn view_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Настройки");
        ui.add_space(8.0);

        if let Some(cfg) = &self.cfg {
            ui.horizontal(|ui| {
                ui.label("Устройство:");
                ui.label(RichText::new(&cfg.device_name).strong());
                ui.label(
                    RichText::new(format!("({})", cfg.device_id))
                        .color(DIM)
                        .small(),
                );
            });
            if let Some(s) = &self.status {
                ui.label(
                    RichText::new(format!("демон v{}", s.daemon_version))
                        .color(DIM)
                        .small(),
                );
            }
            ui.add_space(8.0);
        }

        egui::Frame::group(ui.style()).fill(PANEL).show(ui, |ui| {
            ui.label(RichText::new("Бэкапы").strong());
            ui.horizontal(|ui| {
                ui.label("Папка бэкапов:");
                ui.add(egui::TextEdit::singleline(&mut self.form.backups_dir).desired_width(420.0));
            });
            ui.horizontal(|ui| {
                ui.label("Хранить копий на игру:");
                ui.add(egui::TextEdit::singleline(&mut self.form.backups_keep).desired_width(60.0));
                ui.label(RichText::new("пусто — по умолчанию 10").color(DIM).small());
            });
            ui.horizontal(|ui| {
                ui.label("Авто-бэкап каждые N часов:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.backups_auto_hours)
                        .desired_width(60.0),
                );
                ui.label(RichText::new("пусто — выключено").color(DIM).small());
            });
        });
        ui.add_space(10.0);

        if ui
            .add_enabled(!self.busy(), egui::Button::new("Сохранить"))
            .clicked()
        {
            self.save_config();
        }
    }
}
