# Changelog

Формат — [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование — [SemVer](https://semver.org/lang/ru/).

## [Unreleased]

### Discovery

- Runtime подключает локальный Steam appinfo/UFS и дополнительные библиотеки;
  реестр Steam Windows, стандартные каталоги Linux и Flatpak, canonical roots.
- Встроенные 17 манифестов с отдельной лицензией и команда `manifests import`
  с полной проверкой, atomic replacement и hot reload через IPC.
- Игры без созданных сейвов видны в каталоге и могут принять удалённую версию;
  пустые snapshots не публикуются. Curated fallback/portability и custom rules
  сохраняются, пересекающиеся правила не дублируют файлы.


### Fixed

- Проверка структуры снапшотов, размеров и агрегированного хеша; запрет
  абсолютных путей, Windows device names, unsafe characters, case collisions
  и симлинков назначения до записи сейвов.
- Переносимые имена `custom:`-игр на Windows, доставка истории во все каналы,
  кеш архивов и миграция доступной истории прежних сборок.
- Сохранение удалённых конфликтных голов, parent от применённой версии,
  явное разрешение конфликта через merge-версию, блокировка явного restore.
- Сопоставление Windows/Proton и явных `location` между платформами;
  неполное сопоставление отклоняется без отметки Applied.
- Пересканирование перед sync/backup, работающий поиск по кнопке «Обновить»,
  проверка ошибок операций в desktop-клиенте и применение LAN-настроек
  к работающему listener.
- Строгая сборка TypeScript Decky, воспроизводимый npm lockfile,
  запуск CLI/IPC smoke на обеих ОС в CI и обработка release tags.

### Changed

- Разделены контроллер приложения, формы, представления и IPC-клиент;
  команды демона и сервисы runtime вынесены в отдельные модули.
- Движок разделён на persistence, upload, restore, import, locks и mapping;
  тестовые suites вынесены из файлов реализации.
- CLI получил `history`, `restore`, изоляцию через `SAVESYNC_SOCKET`
  и ненулевой exit code для неуспешных результатов синхронизации.
- README отражает фактическую незавершённость требований 1.0.0.

### Added

- Скелет cargo workspace: `core`, `daemon`, `app`, `decky-bridge`.
- `savesync-core`: slug-правила (`game_key`) с транслитерацией кириллицы —
  эталонная реализация schema savesync-manifests §3.
- `savesync-core`: парсер и валидация манифестов `savesync-manifests`
  (schema v1), загрузка дерева `manifests/<shard>/<slug>.yaml`.
- `savesync-core`: резолвер плейсхолдеров путей, включая Proton-маппинг
  `<steamCompat>` по Steam-библиотекам.
- `savesync-core`: сканер установленных игр (манифесты × корни системы,
  кроссплатформенная тестовая матрица Win/Linux).
- `savesync-core`: blake3-хеширование файлов и снапшотов.
- `savesync-core::steam`: парсер `appinfo.vdf` (v39–41, string table v41),
  бинарный KeyValues, текстовый VDF (`appmanifest_*.acf`,
  `libraryfolders.vdf`), обнаружение библиотек и Proton-префиксов
  (`compatdata/<appid>`), матчинг установленных игр на манифесты
  (`ids.steam` → fallback на slug имени).
- `AGENTS.md` — роли крейтов и взаимодействие компонентов.
- `savesync-core::snapshot` — иммутабельные zip-снапшоты с `manifest.json`
  (game_key, устройство, blake3-хеши файлов), verify и zip-slip-защита
  при распаковке.
- `savesync-core::index` — локальный/общий индекс версий: ветвления,
  детект конфликтов по «головам», дедупликация по хешу, многосторонний
  merge по id, атомарная запись.
- `savesync-core::backend` — trait `Backend` + `FsBackend` (модель данных
  облака в локальной папке; эталон для тестов и ручного канала через USB).
- `savesync-core::engine` — ручная синхронизация: `push` (снапшот при
  изменении контент-хеша), `pull` (merge индекса → политика конфликтов →
  авто-бэкап перед затиранием → распаковка), откат на конкретную версию.
  Интеграционные тесты «двух устройств» на FsBackend.
- `savesync-core::config` — конфиг демона (config.json), стабильный
  device_id, per-platform data dir; каналы независимы (M1: fs).
- `savesync-core::ipc` — типы JSON-RPC протокола (status/scan/sync).
- `savesync-daemon`: runtime (сканер → engine), команды `serve` / `scan` /
  `sync [game]` / `status`; IPC-сервер JSON-RPC через interprocess
  (namespaced: named pipe на Windows, abstract unix socket на Linux);
  второй инстанс демона отклоняется; sync не создаёт ложных ветвлений —
  устройство без локальных версий принимает удалённую линию.
- Документ `docs/manifests-schema.md` — схема репозитория манифестов
  (формат YAML, правила slug'ов, структура репо, валидация).
- `savesync-core::steam::ufs` — извлечение UFS-конфигов savefiles из
  appinfo (Steam Cloud): маппинг Win-путей на плейсхолдеры схемы и
  Proton-двойники `<steamCompat>/<appid>/pfx/...`.
- `savesync-manifest-gen` — генератор манифестов из `appinfo.vdf`
  (`--appinfo/--out/--filter/--dry-run`); существующие файлы не
  перезаписываются, каждая запись проходит `Manifest::validate`.
- Репозиторий `savesync-manifests` (github.com/Langepas94/savesync-manifests):
  SCHEMA.md, валидатор `tools/validate.py` + CI, blocklist.yaml, 17 сидовых
  манифестов, сгенерированных из реального appinfo.
- `savesync-core::steam::kv_binary`: lossy-декодирование строк (реальный
  appinfo содержит не-UTF8 байты).
- Локи в индексе (plan §2.1): `Index::acquire_lock/release_lock/
  lock_holder` с heartbeat и протуханием (120 с); merge хранит свежайший
  heartbeat; engine блокирует push/pull игры, залоченной другим
  устройством (`PushOutcome::Locked`/`PullOutcome::Locked`); демон
  отражает исход `locked` в SyncResult.
- Custom-правила из CLI (schema §3): `savesync-daemon custom add|list|
  remove`; манифесты `custom:<slug>` хранятся в `<state>/custom/*.yaml`,
  подхватываются сканером и никогда не перезаписываются обновлением базы;
  `Manifest::validate` принимает префикс `custom:` и ловит пустые pattern.
- `savesync-core::yandex` — бэкенд Яндекс.Диска (M2): REST API
  cloud-api.yandex.net (mkdir/upload/download/list), OAuth-токен из
  конфига `backends.yandex.token`, маппинг ошибок (401/403 →
  PermissionDenied, 404 → NotFound); тесты на встроенном mock HTTP-сервере,
  включая полный цикл engine push→pull поверх него.
- Каналы независимы (plan §3): демон гоняет sync по всем настроенным
  бэкендам (`fs`, `yandex-disk`), в detail результата — имя канала;
  `Backend` реализован для `Box<dyn Backend>`.
- CLI `remote setup fs|yandex [off]` и `remote list` — настройка каналов
  без правки конфига руками.
- `savesync-core::gdrive` — бэкенд Google Drive (M2): Drive API v3,
  скрытая область `appDataFolder`, резолв путей name→file id с кешем,
  перезапись без дубликатов; access token из конфига
  `backends.google.token` (refresh-flow — после получения client
  id/secret). Mock-тесты: roundtrip API + engine поверх mock-облака;
  CLI `remote setup google <token>` / `google-off`.
- `savesync-core::lan` — LAN-канал (M3, plan §3): TCP-протокол с кадрами
  `[u32 len][payload]`, JSON-заголовки + бинарные blob'ы; сервер
  обслуживает `<state>/lan-store` для пиров, клиент `LanBackend`
  реализует `Backend`; pairing PIN (на сервере — blake3-хеш, неверный
  PIN без побочных эффектов). Шифрование трафика — итерация 2 (Noise/TLS).
- Демон: `serve` поднимает LAN-сервер при `backends.lan`; каналы LAN
  (своё хранилище + пиры) участвуют в sync наравне с облаком; CLI
  `remote setup lan <port> <pin>`, `remote peer add|remove <host:port>`.
- `savesync-core::backup` — пользовательские бэкапы (M6, plan §2.2):
  переносимые файлы `*.savesync-backup` (самодостаточный zip-снапшот),
  `create/list/prune`; `Engine::import_backup` регистрирует файл как
  новую версию в линии игры (с дедупликацией по контент-хешу).
- Retention авто-бэкапов движка перед затиранием (по умолчанию 10,
  `Engine::with_backup_retention`).
- CLI `backup create|list|import|dir|auto`; в `serve` — тред авто-бэкапов
  по расписанию (`backups.auto_interval_hours`, проверка раз в 30 мин).
- CI (GitHub Actions): fmt, clippy, test на Windows и Ubuntu.
- `savesync-app` — десктопное приложение (M4, eframe/egui, палитра Steam):
  список игр со статусами, синхронизация всех/одной игры в фоне
  (спиннер, без блокировки UI), детали игры с историей версий и
  применением любой версии, конфликт-резолвер (выбор ветки при
  нескольких «головах»), кнопка переносимого бэкапа, экраны
  «Подключения» (fs/Яндекс/Google/LAN) и «Настройки» (бэкапы,
  устройство) через SaveConfig; баннер с подсказкой, когда демон не
  запущен.
- `savesync-core::ipc`: методы `history`/`restore`/`backup`/`get_config`/
  `save_config` и клиентский модуль `ipc::client` (`call`/`probe`,
  `DEFAULT_SOCKET`) — общий для приложения, CLI и будущего Decky-прокси.
- Демон: runtime-методы `history`, `restore`, `backup_game`,
  `get_config`, `save_config` (сохраняет device_id/name), IPC-диспетчер
  покрывает все методы приложения.
- `decky-plugin/` — плагин Decky Loader (M5): Python-прокси `main.py`
  к unix-сокету демона (abstract namespace, тот же JSON-RPC), React-шторка
  Quick Access 360 px (`index.tsx`): статус демона, кнопка
  «Синхронизировать», баннер конфликта (резолвер — в приложении), список
  игр; `plugin.json`/`package.json`/rollup-сборка. Тесты прокси
  (unittest, фейковый unix-сервер) — в CI на Ubuntu.
- `packaging/systemd/savesync.service` — systemd user unit демона
  (Restart=on-failure, after network-online).
- `packaging/install-steamos.sh` — установка на SteamOS в home-каталог
  (бинарь в `~/.local/bin`, enable --now сервиса), без sudo и без
  `steamos-readonly disable`.
