use super::theme::*;
use crate::application::Application;
use eframe::egui::{self, RichText};

impl Application {
    pub(crate) fn view_cloud(&mut self, ui: &mut egui::Ui) {
        ui.heading("Облако");
        ui.label("Синхронизация через интернет");
        ui.add_space(8.0);

        if let Some(provider) = self.oauth_pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    if provider == savesync_core::auth::oauth::Provider::Yandex {
                        "Завершите вход в браузере. Затем SaveSync проверит чтение и запись."
                    } else {
                        "Завершите вход в браузере"
                    },
                );
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
                ui.heading("Яндекс Диск");
                let configured = self.cfg.as_ref().is_some_and(|cfg| cfg.backends.yandex.is_some());
                let available = !self.busy() && self.oauth_pending.is_none() && self.yandex_check.is_none();
                if let Some((notice, error)) = &self.yandex_notice {
                    ui.colored_label(if *error { WARN } else { OK }, notice);
                } else if self.yandex_check.is_some() {
                    ui.label("Проверяем чтение и запись…");
                } else if configured {
                    ui.label("Аккаунт подключён. Доступ можно проверить.");
                } else {
                    ui.label("Аккаунт не подключён.");
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(available, egui::Button::new(if configured { "Войти заново" } else { "Войти через Яндекс" })).clicked() {
                        self.begin_oauth(savesync_core::auth::oauth::Provider::Yandex);
                    }
                    if configured && ui.add_enabled(available, egui::Button::new("Проверить доступ")).clicked() {
                        self.check_yandex_access();
                    }
                });
                ui.collapsing("Настройка OAuth и ручной токен", |ui| {
                    ui.label("Client ID своего приложения Яндекса");
                    ui.add(egui::TextEdit::singleline(&mut self.yandex_client_id).desired_width(320.0));
                    ui.small("Redirect URI для стандартного порта");
                    ui.monospace("http://127.0.0.1:17653/oauth/callback");
                    ui.hyperlink_to("Как настроить OAuth-приложение", "https://yandex.ru/dev/id/doc/ru/codes/code-url");
                    if configured {
                        ui.small("Вход через браузер использует папку приложения. Старый ручной токен может работать с отдельной папкой на Диске.");
                    }
                    ui.label("OAuth-токен");
                    ui.add(egui::TextEdit::singleline(&mut self.form.yandex_token).password(true).desired_width(320.0));
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
                    self.begin_oauth(savesync_core::auth::oauth::Provider::Google);
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

            ui.add_space(10.0);

            if ui
                .add_enabled(!self.busy(), egui::Button::new("Сохранить"))
                .clicked()
            {
                self.save_config();
            }
        });
    }
}
