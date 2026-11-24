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
- Документ `docs/manifests-schema.md` — схема репозитория манифестов
  (формат YAML, правила slug'ов, структура репо, валидация).
- CI (GitHub Actions): fmt, clippy, test на Windows и Ubuntu.
