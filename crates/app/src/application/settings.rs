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
                    });
                    ui.horizontal(|ui| {
                        ui.label("Пиры (host:port, через запятую):");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.lan_peers)
                                .desired_width(320.0),
                        );
                    });
                    self.view_lan_pairing(ui);
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
        let busy = self.busy();
        self.updates.view(ui, busy);
        ui.separator();
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

impl SaveSyncApp {
    fn view_lan_pairing(&mut self, ui: &mut egui::Ui) {
        use savesync_core::lan::LanAction;
        let busy = self.busy();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Найти устройства"))
                .clicked()
            {
                self.lan_action(LanAction::Discover);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Показать PIN на 2 минуты"))
                .clicked()
            {
                self.lan_action(LanAction::Invite);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Обновить доверие"))
                .clicked()
            {
                self.lan_action(LanAction::Status);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Адрес:");
            ui.text_edit_singleline(&mut self.lan_address);
            ui.label("PIN:");
            ui.add(
                egui::TextEdit::singleline(&mut self.lan_pair_pin)
                    .password(true)
                    .desired_width(75.0),
            );
            if ui
                .add_enabled(!busy, egui::Button::new("Сопрячь"))
                .clicked()
            {
                let pin = std::mem::take(&mut self.lan_pair_pin);
                self.lan_action(LanAction::Pair {
                    address: self.lan_address.trim().into(),
                    pin,
                });
            }
        });
        if let Some(status) = self.lan.clone() {
            if let Some(pin) = status.invitation_pin {
                ui.label(format!(
                    "PIN: {pin}. Введите на другом устройстве. Не используйте повторно."
                ));
                if ui
                    .add_enabled(!busy, egui::Button::new("Закрыть приглашение"))
                    .clicked()
                {
                    self.lan_action(LanAction::ClosePairing);
                }
            }
            for peer in status.discovered {
                ui.horizontal(|ui| {
                    ui.label(format!("{} — {}", peer.name, peer.address));
                    if ui.button("Выбрать адрес").clicked() {
                        self.lan_address = peer.address;
                    }
                });
            }
            for entry in status.peers {
                let id = entry.peer.id.clone();
                if entry.confirmed {
                    ui.horizontal(|ui| {
                        ui.label(format!("Доверенное устройство {}", &id[..12]));
                        if ui.add_enabled(!busy, egui::Button::new("Забыть")).clicked() {
                            self.lan_action(LanAction::Forget { peer_id: id });
                        }
                    });
                } else {
                    ui.label(format!(
                        "Устройство {}. SAS: {}",
                        &id[..12],
                        entry.sas.unwrap_or_default()
                    ));
                    ui.label(
                        "Сравните SAS на обоих устройствах. Подтверждайте только при совпадении.",
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("SAS совпадает — доверять"))
                            .clicked()
                        {
                            self.lan_action(LanAction::Confirm {
                                peer_id: id.clone(),
                            });
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("Отклонить"))
                            .clicked()
                        {
                            self.lan_action(LanAction::Forget { peer_id: id });
                        }
                    });
                }
            }
        }
    }
}
