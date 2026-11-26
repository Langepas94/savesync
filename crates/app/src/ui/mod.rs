//! Отрисовка окна и экранов SaveSync.

use eframe::egui::{self, RichText};

use crate::application::{Application, Tab};

mod backups;
mod cloud;
mod connections;
mod game_details;
mod games;
pub(crate) mod presentation;
mod settings;
pub(crate) mod theme;
mod updates;

use theme::*;

impl Application {
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        start_daemon: impl FnOnce() -> std::io::Result<()>,
    ) {
        // Верхняя панель: состояние демона и доступные устройства.
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SaveSync").size(20.0).color(ACCENT).strong());
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.daemon_ok {
                        let label = if self.status.as_ref().is_some_and(|status| status.busy) {
                            "● выполняется операция"
                        } else {
                            "● демон работает"
                        };
                        ui.label(RichText::new(label).color(OK).small());
                        if let Some(status) = &self.lan {
                            let online: Vec<_> = status
                                .peers
                                .iter()
                                .filter(|peer| {
                                    peer.confirmed
                                        && self
                                            .online_peers
                                            .as_ref()
                                            .is_some_and(|ids| ids.contains(&peer.peer.id))
                                })
                                .collect();
                            let connection = match online.as_slice() {
                                [peer] => Some(format!(
                                    "{} в сети",
                                    peer.peer.name.as_deref().unwrap_or("Устройство")
                                )),
                                [] => None,
                                _ => Some(format!("Устройств в сети: {}", online.len())),
                            };
                            if let Some(connection) = connection {
                                if ui.link(RichText::new(connection).color(OK)).clicked() {
                                    self.tab = Tab::Connections;
                                    self.poll_lan();
                                }
                            }
                        }
                    } else {
                        ui.label(RichText::new("● демон не запущен").color(WARN).small());
                    }
                });
            });
            ui.add_space(4.0);
        });

        egui::SidePanel::left("navigation")
            .exact_width(150.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                for (tab, label) in [
                    (Tab::Games, "Игры"),
                    (Tab::Connections, "Устройства"),
                    (Tab::Cloud, "Облако"),
                    (Tab::Backups, "Бэкапы"),
                    (Tab::Settings, "Настройки"),
                ] {
                    if ui
                        .add_sized(
                            [ui.available_width(), 33.0],
                            egui::SelectableLabel::new(self.tab == tab, label),
                        )
                        .clicked()
                    {
                        self.tab = tab;
                        if tab == Tab::Connections {
                            self.poll_lan();
                        }
                    }
                    ui.add_space(3.0);
                }
            });

        // Нижняя панель: уведомления / прогресс.
        egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(description) = self.job_description() {
                    ui.spinner();
                    ui.label(description);
                } else if let Some((msg, is_err)) = &self.notice {
                    let color = if *is_err { WARN } else { OK };
                    if msg.lines().count() > 1 {
                        ui.menu_button("Подробности", |ui| {
                            ui.set_max_width(520.0);
                            egui::ScrollArea::vertical()
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    ui.add(egui::Label::new(msg).wrap());
                                });
                        });
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(msg.lines().next().unwrap_or(msg))
                                .color(color)
                                .small(),
                        )
                        .truncate(),
                    )
                    .on_hover_text(msg);
                } else {
                    ui.label(RichText::new("готов").color(DIM).small());
                }
            });
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.daemon_ok && self.tab != Tab::Settings {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Демон не запущен").size(18.0).color(WARN));
                    ui.add_space(8.0);
                    if ui.button("Запустить SaveSync").clicked() {
                        if let Err(error) = start_daemon() {
                            self.set_notice(error.to_string(), true);
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Приложение подключится автоматически.").color(DIM));
                });
                self.probe_daemon();
                return;
            }
            match self.tab {
                Tab::Games => self.view_games(ui),
                Tab::Connections => {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| self.view_connections(ui))
                        .inner
                }
                Tab::Cloud => self.view_cloud(ui),
                Tab::Backups => self.view_backups(ui),
                Tab::Settings => self.view_settings(ui),
            }
        });
    }
}
