# Changelog

Формат — [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование — [SemVer](https://semver.org/lang/ru/).

## [Unreleased]

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
- CI (GitHub Actions): fmt, clippy, test на Windows и Ubuntu.
