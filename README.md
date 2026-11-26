# SaveSync

**Готовые сборки в этой рабочей папке:** [Windows](../СБОРКИ/Windows/savesync-setup-dev.exe),
[Steam Deck](../СБОРКИ/SteamDeck/savesync-steamos.tar.gz),
[инструкция установки](../СБОРКИ/НАЧАТЬ-ЗДЕСЬ.md).
Это одна папка `СБОРКИ` рядом с каталогом исходников `savesync`.
Предыдущие локальные пакеты убраны в `dist/archive/`.
При просмотре репозитория на GitHub пакеты доступны в [артефактах проверенной сборки 2f3ed62](https://github.com/Langepas94/savesync/actions/runs/45659135205#artifacts).

Кроссплатформенный клиент синхронизации сохранений игр между Windows PC и
Steam Deck (SteamOS). Аналог Steam Cloud, но: любые игры (не только Steam),
версионирование, конфликты, бэкапы, каналы синхронизации — Яндекс.Диск /
Google Drive и/или прямая LAN-синхронизация.

**Статус:** кандидат 1.0.0. Код требований реализован и проверяется CI;
публичный подписанный релиз ожидает внешних gates: сертификат издателя,
зарегистрированные OAuth-приложения и проверка физического Steam Deck.
[Готовность кандидата](docs/RELEASE-1.0.0.md).
Реализованы ручная синхронизация, история, конфликты, бэкапы, desktop UI
и Decky-прокси. Windows-доставка реализована; подпись release требует сертификат владельца
([настройка](docs/WINDOWS-DELIVERY.md)). Конкурентные облачные версии
сохраняются журналом immutable publications ([описание](docs/INDEX-PUBLICATIONS.md)). Защищённый LAN с SPAKE2/QUIC и подтверждением SAS готов
([сопряжение](docs/LAN.md)). Настраиваемый OAuth реализован; живой вход требует
регистрации приложений Google/Яндекс. См. [QA-отчёт](docs/QA-2026-11-25.md).
Автотриггеры синхронизации относятся к итерации 2.

## Пользователю

- [Установить на Windows и Steam Deck и проверить перенос туда-обратно](docs/INSTALL-AND-TEST.md) — начните здесь.
- [Первый запуск за несколько шагов](docs/QUICKSTART.md).
- [Как переносить сейвы, настроить LAN и разрешить конфликт](docs/USER-GUIDE.md).
- [Проверка 1.0.0 на тестовом файле и шаблон обратной связи](docs/TRY-1.0.0.md).

## Архитектура

Desktop и Decky обращаются к демону по локальному JSON-RPC. Демон также поддерживает
запуск части команд CLI в офлайн-режиме. Реализация синхронизации находится в `core`.

| Задача | Где искать |
|---|---|
| Экран, навигация, виджеты | [`app/src/ui`](crates/app/src/ui) |
| Действия desktop, фоновые задачи и состояние | [`app/src/application`](crates/app/src/application) |
| Окно, экземпляр приложения, запуск демона и трей | [`app/src/desktop`](crates/app/src/desktop) |
| Команды CLI и формат консольного вывода | [`daemon/src/cli`](crates/daemon/src/cli) |
| IPC-сервер и преобразование результатов в ответы | [`daemon/src/ipc`](crates/daemon/src/ipc) |
| Сценарии демона: синхронизация, история, бэкапы, подключения | [`daemon/src/service`](crates/daemon/src/service) |
| Движок, восстановление и индекс версий | [`core/src/sync`](crates/core/src/sync) |
| Манифесты, сопоставление игры и сканирование | [`core/src/catalog`](crates/core/src/catalog) |
| Steam, лаунчеры, префиксы, платформенные пути | [`core/src/platform`](crates/core/src/platform) |
| Снапшоты, архивы бэкапов и пути данных SaveSync | [`core/src/storage`](crates/core/src/storage) |
| Файловый backend, Yandex, Google Drive и LAN | [`core/src/backends`](crates/core/src/backends) |
| OAuth и учётные данные | [`core/src/auth`](crates/core/src/auth) |
| Формат сообщений JSON-RPC, клиент и проверка локального соединения | [`core/src/ipc`](crates/core/src/ipc) |
| Проверка и установка обновлений | [`core/src/updates.rs`](crates/core/src/updates.rs) |
| Генерация манифестов из Steam appinfo | [`manifest-gen`](crates/manifest-gen) |
| Панель Decky, Python-прокси и его тесты | [`decky-plugin`](decky-plugin) |

`core` не зависит от приложений. В desktop слой `application` работает через `DaemonClient`
без зависимости от `egui`. В демоне CLI и IPC используют общий преобразователь команд,
а `service` возвращает типизированные результаты операций. Объявления пакетов Cargo
и старые публичные пути модулей `savesync_core` поддерживаются для совместимости.
[`decky-bridge`](crates/decky-bridge) содержит Rust-типы. Рабочий Decky-плагин использует
свой Python-прокси.

Типовые цепочки:

- Синхронизация. `app/ui/games.rs` вызывает действие в `app/application/games.rs`.
  Запрос проходит через `daemon/ipc` к `daemon/service/sync.rs` и движку `core/sync/engine`.
- Восстановление. Карточка игры в `app/ui/game_details.rs` вызывает сценарий
  `daemon/service/history.rs`. Запись сейвов выполняет `core/sync/engine/restore.rs`.
- LAN. Экран `app/ui/connections.rs` использует действия из `app/application/connections.rs`.
  Демон выполняет их в `daemon/service/networking.rs` через `core/backends/lan`.

Пути в цепочках указаны от `crates/` с опущенным каталогом `src/`.

Идентификация игр: `game_key` = slug из собственного репозитория
[`savesync-manifests`](docs/manifests-schema.md) + локальный парсер Steam
appinfo + ручные правила (`custom:<slug>`). Чужих API в рантайме нет.

## Разработка

Требования: Rust 1.89+ (`rustup`), git. Процесс — Git Flow
(см. [HANDOFF](docs/HANDOFF.md) §2): `main` (релизы, теги) / `develop` /
`feature/*` через PR. Conventional Commits, CI: `fmt`, `clippy -D warnings`,
`cargo test` (matrix: Windows + Linux).

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
python scripts/smoke.py  # реальные CLI + IPC, изолированные профили, без GUI
```

### Размещение тестов

Unit-тесты лежат в `crates/<crate>/src/tests/`. Имена файлов соответствуют
подсистемам. Модули подключают приватные тесты через `#[cfg(test)]` и `#[path]`.
Интеграционные проверки публичного контракта лежат в `crates/<crate>/tests/`.
`scripts/smoke.py` проверяет CLI и IPC через запущенный бинарник.

### Демон

Конфиг: `%APPDATA%\savesync\config.json` / `~/.local/share/savesync/config.json`
(создаётся при первом запуске; `manifests_dir`, `backends.fs_path`).

```sh
savesync-daemon serve        # фоновый демон (IPC: named pipe / unix socket)
savesync-daemon scan         # найти игры
savesync-daemon sync [игра]  # ручная синхронизация (всех или одной)
savesync-daemon status       # статус демона
savesync-daemon history <game_key>
savesync-daemon restore <game_key> <version_id>
```

`SAVESYNC_SOCKET` задаёт отдельное IPC-имя для изолированного профиля
(по умолчанию `savesync-daemon`). Конфликт, блокировка и ошибка синхронизации
возвращают ненулевой exit code. Данные профиля задаются стандартными
`APPDATA` / `XDG_DATA_HOME`; скрипт smoke не меняет пользовательский профиль.

### Восстановление

Выбор старой версии создаёт новую запись истории: следующий sync сохраняет
результат отката. Restore сначала готовит файлы и safety backups, затем
фиксирует изменения с журналом и откатывает их при ошибке. Прерванная операция
восстанавливается при следующем запуске. Ранее tracked файлы, отсутствующие
в выбранной версии, удаляются с бэкапом; untracked файлы сохраняются.
Если локальный commit завершён, а канал недоступен, повторите sync для публикации.

### Поиск игр и база манифестов

Steam обнаруживается через HKCU/стандартные каталоги Windows и стандартные,
Flatpak-каталоги Linux. `libraryfolders.vdf` подключает дополнительные диски;
`appinfo.vdf` даёт UFS-правила установленных игр. Curated fallback-пути и
ограничения переносимости сохраняются. База из 17 игр встроена в бинарь
([лицензия базы](crates/core/manifests/LICENSE)); для её обновления:

```sh
savesync-daemon manifests import /путь/к/savesync-manifests
```

Вся база проверяется перед заменой. Custom-правила хранятся отдельно и
сохраняются. Запущенный демон перечитывает базу через IPC без перезапуска.
`SAVESYNC_STEAM_ROOT` позволяет указать нестандартный корень Steam или
изолировать тестовый профиль.

Установленная игра появляется и до создания сейвов. Sync не публикует пустую
версию: при наличии удалённой истории он скачивает сейв, иначе возвращает
`no_saves`. Пересекающиеся правила архивируют каждый файл один раз.

## Установка

> Ниже описан ручной запуск сборки. Готовый подписанный установщик и релиз
> ещё не опубликованы.

### Windows

1. Для подписанного release используйте `savesync-setup.exe`: установка в профиль
   пользователя, без SYSTEM-службы. До настройки сертификата доступны только
   `savesync-setup-dev.exe` и сборка из исходников; dev package не является релизом.
2. Установщик предлагает HKCU autostart. Приложение запускает daemon скрыто;
   окно закрывается в трей. Повторный запуск открывает существующее окно.
3. «Подключения» — OAuth ([настройка](docs/OAUTH.md)), LAN с SAS
   ([сопряжение](docs/LAN.md)) либо локальные бэкапы.
4. «Настройки → Проверить Releases»: подписанное обновление, проверка publisher
   pin, версии и SHA256. Upgrade ждёт завершения sync. Для приватного репозитория
   нужен read-only `SAVESYNC_GITHUB_TOKEN`.

[Сборка, Secrets, uninstall и ограничения](docs/WINDOWS-DELIVERY.md).

### Steam Deck (SteamOS)

1. Desktop Mode → `packaging/install-steamos.sh`: ставит daemon и приложение в
   `~/.local/bin`, регистрирует `systemctl --user enable --now
   savesync.service` (unit — `packaging/systemd/savesync.service`).
2. (Опционально) Decky Loader → плагин SaveSync из `decky-plugin/`:
   шторка в Quick Access (статус, кнопка «Синхронизировать», баннер
   конфликта; резолвер конфликтов — в десктопном приложении).
3. Установка только в home, без `sudo steamos-readonly disable`. Обновление
   сначала останавливает daemon/app, затем заменяет staged binaries. При ошибке
   возвращает прежний пакет. Профиль сохраняется.
4. Для Gaming Mode доступны Secret Service или [шифрованный vault](docs/OAUTH.md).
   Read-only проверка настоящего устройства: `packaging/verify-steamos.sh`.
   Реальные OAuth, два сетевых хоста и Decky QAM остаются физическим gate.

### Общее

- Требования: Windows 10+, SteamOS 3.x.
- Обновление: Windows — проверяемый installer из UI; SteamOS — userspace install script.
- Удаление: остановить демон/сервис, удалить данные
  `~/.local/share/savesync` / `%APPDATA%\savesync`.

## Документы

- [HANDOFF](docs/HANDOFF.md) — точка входа: требования, процесс, дорожная карта
- [План](docs/plan.md) — полная архитектура и стратегия синхронизации
- [Ресерч рисков](docs/research/RESEARCH.md)
- [Схема savesync-manifests](docs/manifests-schema.md)
- [Дизайн](docs/design/) — мокапы desktop-приложения и Decky-шторки
- [CHANGELOG](CHANGELOG.md)
- [Структура кода](docs/ARCHITECTURE.md)
- [QA и блокеры 1.0.0](docs/QA-2026-11-25.md)
