# SaveSync

Кроссплатформенный клиент синхронизации сохранений игр между Windows PC и
Steam Deck (SteamOS). Аналог Steam Cloud, но: любые игры (не только Steam),
версионирование, конфликты, бэкапы, каналы синхронизации — Яндекс.Диск /
Google Drive и/или прямая LAN-синхронизация.

**Статус:** M1–M6 завершены (core, демон, десктопное приложение, Decky-плагин,
бэкапы). Ручная синхронизация — без таймеров и автодетекта игр
(автотриггеры — итерация 2).

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

### Демон (M1)

Конфиг: `%APPDATA%\savesync\config.json` / `~/.local/share/savesync/config.json`
(создаётся при первом запуске; `manifests_dir`, `backends.fs_path`).

```sh
savesync-daemon serve        # фоновый демон (IPC: named pipe / unix socket)
savesync-daemon scan         # найти игры
savesync-daemon sync [игра]  # ручная синхронизация (всех или одной)
savesync-daemon status       # статус демона
```

## Установка

> Разделы наполняются по ходу M4/M5 (см. HANDOFF §4).

### Windows

1. Скачать архив `savesync-x86_64-pc-windows-msvc` из Releases
   (демон + десктопное приложение). Подпись кода пока отсутствует —
   Windows SmartScreen может показать предупреждение.
2. Автозапуск демона — добавить в `HKCU\Software\Microsoft\Windows\
   CurrentVersion\Run` команду `savesync-daemon serve`
   (user-context, не SYSTEM-сервис).
3. Запустить `savesync-app.exe`: экран «Подключения» — токен Яндекс.Диска
   и/или Google Drive, LAN-pairing по PIN, либо только локальные бэкапы.

### Steam Deck (SteamOS)

1. Desktop Mode → `packaging/install-steamos.sh`: кладёт бинарь в
   `~/.local/bin`, регистрирует `systemctl --user enable --now
   savesync.service` (unit — `packaging/systemd/savesync.service`).
2. (Опционально) Decky Loader → плагин SaveSync из `decky-plugin/`:
   шторка в Quick Access (статус, кнопка «Синхронизировать», баннер
   конфликта; резолвер конфликтов — в десктопном приложении).
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
