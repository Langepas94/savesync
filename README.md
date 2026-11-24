# SaveSync

Кроссплатформенный клиент синхронизации сохранений игр между Windows PC и
Steam Deck (SteamOS). Аналог Steam Cloud, но: любые игры (не только Steam),
версионирование, конфликты, бэкапы, каналы синхронизации — Яндекс.Диск /
Google Drive и/или прямая LAN-синхронизация.

**Статус:** M1 (core MVP). Только ручная синхронизация — без таймеров,
опросов облака и автодетекта игр (автотриггеры — итерация 2).

## Архитектура

Единый Rust-демон со всей логикой + тонкие UI по IPC:

| Крейт | Назначение |
|---|---|
| `crates/core` (`savesync-core`) | slug'и, манифесты, сканер, хеширование, снапшоты, индекс |
| `crates/daemon` (`savesync-daemon`) | sync engine, бэкенды (YandexDisk / GoogleDrive / LAN), IPC-сервер |
| `crates/app` (`savesync-app`) | десктопное приложение (Windows / SteamOS Desktop Mode), M4 |
| `crates/decky-bridge` (`savesync-decky-bridge`) | типы JSON-RPC для Decky-плагина, M5 |

Идентификация игр: `game_key` = slug из собственного репозитория
[`savesync-manifests`](docs/manifests-schema.md) + локальный парсер Steam
appinfo + ручные правила (`custom:<slug>`). Чужих API в рантайме нет.

## Разработка

Требования: Rust 1.85+ (`rustup`), git. Процесс — Git Flow
(см. [HANDOFF](docs/HANDOFF.md) §2): `main` (релизы, теги) / `develop` /
`feature/*` через PR. Conventional Commits, CI: `fmt`, `clippy -D warnings`,
`cargo test` (matrix: Windows + Linux).

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Установка

> Разделы наполняются по ходу M4/M5 (см. HANDOFF §4).

### Windows

1. Скачать `savesync-setup.exe` (подписанный) из Releases.
2. Установщик кладёт демон + приложение, регистрирует автозапуск в HKCU Run
   (user-context, не SYSTEM-сервис).
3. При первом старте — экран «Подключения»: облако (OAuth в браузере) и/или
   LAN-pairing по PIN, либо «пропустить» (локальные бэкапы).

### Steam Deck (SteamOS)

1. Desktop Mode → установщик `.sh`: кладёт бинарь в `~/.local/bin`,
   регистрирует `systemctl --user enable --now savesync.service`.
2. (Опционально) Decky Loader → плагин SaveSync: шторка в Quick Access.
3. Установка только в home, без `sudo steamos-readonly disable`.

### Общее

- Требования: Windows 10+, SteamOS 3.x.
- Обновление: проверка Releases, in-app updater.
- Удаление: остановить демон/сервис, удалить данные
  `~/.local/share/savesync` / `%APPDATA%\savesync`.

## Документы

- [HANDOFF](docs/HANDOFF.md) — точка входа: требования, процесс, дорожная карта
- [План](docs/plan.md) — полная архитектура и стратегия синхронизации
- [Ресерч рисков](docs/research/RESEARCH.md)
- [Схема savesync-manifests](docs/manifests-schema.md)
- [Дизайн](docs/design/) — мокапы desktop-приложения и Decky-шторки
- [CHANGELOG](CHANGELOG.md)
