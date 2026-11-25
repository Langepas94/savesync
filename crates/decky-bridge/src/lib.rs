//! Мост Decky-плагина: типы сообщений JSON-RPC между Python-прокси
//! Decky и savesync-daemon. Сам плагин — `decky-plugin/` (React-шторка
//! + Python-прокси, M5); дизайн-референс: docs/design/decky-panel.html.

use serde::{Deserialize, Serialize};

/// Статус синхронизации одной игры для шторки Decky (360 px: только
/// статус, кнопка «Синхронизировать», баннер конфликта).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSyncStatus {
    pub game_key: String,
    pub display_name: String,
    pub state: SyncState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Локальная версия совпадает с вершиной индекса.
    Synced,
    /// Есть что выгрузить или подтянуть (ручной триггер, MVP).
    Pending,
    /// Ветвление версий: в шторке только баннер, резолвер — в приложении.
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrips_json() {
        let status = GameSyncStatus {
            game_key: "elden-ring".into(),
            display_name: "Elden Ring".into(),
            state: SyncState::Conflict,
        };
        let json = serde_json::to_string(&status).unwrap();
        let back: GameSyncStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.state, SyncState::Conflict);
    }
}
