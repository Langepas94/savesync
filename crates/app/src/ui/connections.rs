use super::theme::*;
use crate::application::Application;
use eframe::egui::{self, RichText};
use savesync_core::backends::lan::LanAction;

impl Application {
    pub(crate) fn view_connections(&mut self, ui: &mut egui::Ui) {
        ui.heading("Устройства");
        self.view_lan(ui);
    }
}

impl Application {
    fn view_lan(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Синхронизация по локальной сети").strong());
        let busy = self.busy();
        let Some(config) = &self.cfg else {
            ui.label("Загрузка настроек LAN…");
            return;
        };
        let enabled = config.backends.lan.is_some();
        let device_name = config.device_name.clone();
        ui.label(format!("Это устройство: {device_name}"));

        let status = self.lan.clone();
        let peers = status.as_ref().map(|status| status.peers.as_slice());
        ui.add_space(6.0);
        ui.label(RichText::new("Подключённые устройства").strong());
        if let Some(peers) = peers {
            let mut trusted = peers.iter().filter(|entry| entry.confirmed).peekable();
            if trusted.peek().is_none() {
                ui.label("Подключённых устройств пока нет.");
            }
            for entry in trusted {
                let label = if entry.peer.name.is_none() {
                    self.lan_discovered
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .find(|found| found.identity == entry.peer.id)
                        .map(|found| format!("{} (имя в сети)", found.name))
                } else {
                    None
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(label.unwrap_or_else(|| Self::lan_peer_label(&entry.peer)))
                        .on_hover_text(&entry.peer.id);
                    match self.online_peers.as_ref() {
                        Some(ids) if ids.contains(&entry.peer.id) => {
                            ui.colored_label(OK, "В сети");
                        }
                        Some(_) if enabled => {
                            ui.colored_label(DIM, "Не в сети");
                        }
                        _ if enabled => {
                            ui.colored_label(DIM, "Проверяем подключение");
                        }
                        _ => {
                            ui.colored_label(DIM, "Локальная сеть выключена");
                        }
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Забыть")).clicked() {
                        self.lan_action(LanAction::Forget {
                            peer_id: entry.peer.id.clone(),
                        });
                    }
                });
            }
        } else {
            ui.label("Загрузка списка устройств…");
        }

        if enabled && status.as_ref().is_some_and(|status| status.port.is_none()) && !busy {
            ui.colored_label(
                WARN,
                "LAN-сервер не запустился. Проверьте порт в расширенных настройках.",
            );
        }
        for entry in peers.into_iter().flatten().filter(|entry| !entry.confirmed) {
            ui.separator();
            ui.label(
                RichText::new(format!("Подтвердите {}", Self::lan_peer_label(&entry.peer)))
                    .strong(),
            );
            ui.label(format!(
                "Код проверки: {}",
                entry.sas.as_deref().unwrap_or_default()
            ));
            ui.label("Сравните этот код на обоих устройствах и подтвердите на каждом.");
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Коды совпадают"))
                    .clicked()
                {
                    self.lan_action(LanAction::Confirm {
                        peer_id: entry.peer.id.clone(),
                    });
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Отклонить"))
                    .clicked()
                {
                    self.lan_action(LanAction::Reject {
                        peer_id: entry.peer.id.clone(),
                    });
                }
            });
        }

        ui.separator();
        ui.label(RichText::new("Добавить устройство").strong());
        let invitation = status
            .as_ref()
            .and_then(|status| status.invitation_pin.as_ref());
        if let Some(pin) = invitation {
            ui.label(RichText::new(format!("PIN: {pin}")).size(20.0).strong());
            ui.label("На втором устройстве найдите это устройство и введите PIN. Он действует до двух минут.");
            if let Some(status) = &status {
                if !status.invitation_addresses.is_empty() {
                    ui.label("Адреса этого устройства для ручного подключения");
                }
                for address in &status.invitation_addresses {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(address);
                        if ui.small_button("Скопировать адрес").clicked() {
                            ui.output_mut(|output| output.copied_text = address.clone());
                        }
                    });
                }
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Закрыть приглашение"))
                .clicked()
            {
                self.lan_action(LanAction::ClosePairing);
            }
        } else {
            ui.label(
                "Покажите PIN на одном устройстве. На втором найдите первое и введите его PIN.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Показать PIN"))
                    .clicked()
                {
                    self.begin_lan_setup(LanAction::Invite);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Найти устройства"))
                    .clicked()
                {
                    self.begin_lan_setup(LanAction::Discover);
                }
            });
            self.view_lan_join(ui, busy, peers.unwrap_or_default());
        }

        ui.add_space(6.0);
        ui.collapsing("Расширенные настройки LAN", |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                let mut toggle = enabled;
                if ui.checkbox(&mut toggle, "Включить локальную сеть").changed() {
                    self.apply_lan_settings(toggle);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label("Локальный UDP-порт");
                    ui.add(egui::TextEdit::singleline(&mut self.form.lan_port).desired_width(80.0));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Адреса синхронизации");
                    ui.add(egui::TextEdit::singleline(&mut self.form.lan_peers).desired_width(270.0));
                });
                ui.label("Формат IP:порт через запятую. Адреса доверенных устройств используются автоматически.");
                if ui.button("Применить настройки LAN").clicked() {
                    self.apply_lan_settings(true);
                }
            });
        });
    }

    fn lan_peer_label(peer: &savesync_core::backends::lan::Peer) -> String {
        match (&peer.name, &peer.address) {
            (Some(name), Some(address)) => format!("{name} · {address}"),
            (Some(name), None) => name.clone(),
            (None, Some(address)) => address.clone(),
            (None, None) => format!(
                "Устройство {}",
                peer.id.chars().take(12).collect::<String>()
            ),
        }
    }

    fn view_lan_join(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
        peers: &[savesync_core::backends::lan::PeerStatus],
    ) {
        ui.add_space(6.0);
        ui.label(RichText::new("Устройства поблизости").strong());
        let mut address_selected = false;
        if let Some(found) = &self.lan_discovered {
            if found.is_empty() {
                ui.label("Устройства не найдены. Проверьте, что LAN включён на обоих устройствах, или введите адрес вручную.");
            }
            for peer in found {
                let row = ui.horizontal_wrapped(|ui| {
                    let known = peers.iter().find(|entry| entry.peer.id == peer.identity);
                    if let Some(entry) = known {
                        ui.label(if entry.confirmed {
                            "Подключено"
                        } else {
                            "Ожидает подтверждения"
                        });
                    } else {
                        let selected = self.lan_address == peer.address;
                        let button = egui::Button::new(if selected {
                            "Выбрано"
                        } else {
                            "Ввести PIN"
                        });
                        if ui.add_enabled(!busy, button).clicked() {
                            self.lan_address.clone_from(&peer.address);
                            address_selected = true;
                        }
                    }
                    ui.label(RichText::new(&peer.name).strong());
                    ui.small(&peer.address);
                });
                if std::mem::take(&mut self.scroll_to_lan_results) {
                    row.response.scroll_to_me(None);
                }
            }
        } else {
            ui.small("Нажмите «Найти устройства», чтобы увидеть список.");
        }
        ui.collapsing("Подключить по адресу", |ui| {
            ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.lan_address).hint_text("IP:порт"),
            );
        });
        if self.lan_address.trim().is_empty() {
            return;
        }
        if let Some(peer) = self
            .lan_discovered
            .as_ref()
            .and_then(|found| found.iter().find(|peer| peer.address == self.lan_address))
        {
            ui.label(format!("Подключение к {}", peer.name));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("PIN второго устройства");
            let pin_field = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.lan_pair_pin)
                    .password(true)
                    .desired_width(85.0),
            );
            if address_selected {
                pin_field.request_focus();
            }
            let valid = !self.lan_address.trim().is_empty()
                && self.lan_pair_pin.trim().len() == 6
                && self
                    .lan_pair_pin
                    .trim()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit());
            if ui
                .add_enabled(!busy && valid, egui::Button::new("Подключить"))
                .clicked()
            {
                self.begin_lan_setup(LanAction::Pair {
                    address: self.lan_address.trim().into(),
                    pin: self.lan_pair_pin.trim().into(),
                });
            }
        });
    }
}
