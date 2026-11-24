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
- Документ `docs/manifests-schema.md` — схема репозитория манифестов
  (формат YAML, правила slug'ов, структура репо, валидация).
- CI (GitHub Actions): fmt, clippy, test на Windows и Ubuntu.
