use std::path::PathBuf;
use std::sync::mpsc;

use savesync_core::ipc::{HistoryPayload, Method, ResponsePayload};

use super::Application;

#[derive(Default)]
pub(crate) struct AddGameForm {
    pub(crate) name: String,
    pub(crate) paths: String,
    pub(crate) error: Option<String>,
    pub(crate) picker: Option<mpsc::Receiver<Option<PathBuf>>>,
}

impl Application {
    pub(crate) fn load_history(&mut self, game_key: &str) {
        if self.job.is_some() {
            return;
        }
        self.history = None;
        self.start_job(
            "Загрузка истории…",
            Method::History {
                game_key: game_key.into(),
            },
            |_| (String::new(), false, false),
        );
    }

    pub(super) fn accept_history(&mut self, history: HistoryPayload) {
        if self.selected.as_deref() == Some(history.game_key.as_str()) {
            self.history = Some(history);
        }
    }

    pub(crate) fn refresh_games(&mut self) {
        self.start_job("Поиск игр…", Method::Scan, |payload| {
            let message = if let ResponsePayload::Scan(scan) = payload {
                format!("найдено игр: {}", scan.found)
            } else {
                "поиск завершён".into()
            };
            (message, true, false)
        });
    }

    pub(crate) fn sync_all(&mut self) {
        self.start_job(
            "Синхронизация всех игр…",
            Method::Sync { game_key: None },
            |p| (sync_summary(&p), true, true),
        );
    }

    pub(crate) fn sync_game(&mut self, game_key: &str) {
        self.start_job(
            "Синхронизация…",
            Method::Sync {
                game_key: Some(game_key.to_string()),
            },
            |p| (sync_summary(&p), true, true),
        );
    }

    pub(crate) fn restore(&mut self, game_key: &str, version_id: &str) {
        self.start_job(
            "Восстановление сохранения…",
            Method::Restore {
                game_key: game_key.to_string(),
                version_id: version_id.to_string(),
            },
            |payload| {
                let unchanged = matches!(payload, ResponsePayload::Sync(sync)
                    if sync.results.iter().all(|result| result.outcome != "applied"));
                let message = if unchanged {
                    "Эта копия уже на устройстве"
                } else {
                    "Сохранение восстановлено"
                };
                (message.into(), true, true)
            },
        );
    }

    pub(crate) fn backup(&mut self, game_key: &str) {
        self.start_job(
            "Создание бэкапа…",
            Method::Backup {
                game_key: game_key.to_string(),
            },
            |p| match p {
                ResponsePayload::Backup(b) => (format!("бэкап: {}", b.file), false, false),
                _ => ("бэкап создан".to_string(), false, false),
            },
        );
    }

    pub(crate) fn set_game_hidden(&mut self, game_key: String, hidden: bool) {
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

    pub(crate) fn submit_game(&mut self) {
        let Some(form) = &mut self.add_game else {
            return;
        };
        let name = form.name.trim().to_string();
        let game_key = savesync_core::catalog::game_key::custom_key(&name);
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

pub(crate) fn sync_summary(p: &ResponsePayload) -> String {
    if let ResponsePayload::Sync(s) = p {
        let mut lines = Vec::new();
        for r in &s.results {
            let mut line = format!("{}: {}", r.game_key, r.outcome);
            if let Some(d) = &r.detail {
                line.push_str(&format!(" ({d})"));
            }
            lines.push(line);
        }
        if lines.is_empty() {
            "нет игр для синхронизации".to_string()
        } else {
            lines.join("\n")
        }
    } else {
        "готово".to_string()
    }
}
