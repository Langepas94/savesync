# SaveSync Agents

## Local coding tools

- Run `ast-index update` from this directory at the start of code work and after edits or branch changes; run `ast-index rebuild` if the cache is missing. `.ast-index.yaml` limits indexing to project source and excludes build artifacts.
- Use `ast-index search <name>`, `ast-index symbol <name>`, `ast-index refs <name>`, and `ast-index outline <file>` for structural navigation. Use `rg` for text and files. Run `ast-index watch` in a separate terminal for live updates when needed.
- Caveman is enabled by default. Read `.agents/skills/caveman/SKILL.md` and apply its concise response style in the user's language, preserving code, commands, paths, errors, and technical meaning. `stop caveman` or `normal mode` disables it for the session. Do not change the model, reasoning effort, or validation requirements.

## Архитектура

Карта подсистем и маршруты для синхронизации, восстановления и LAN находятся
в разделе «Архитектура» [README.md](README.md).

- `crates/core` владеет каталогом, платформенным поиском, движком, хранилищем,
  backend’ами, учётными данными и контрактом IPC. Он не зависит от клиентов и демона.
- `crates/daemon/src/service` владеет сценариями синхронизации и состоянием демона.
  `cli` и `ipc` вызывают сервис через общий диспетчер.
- `crates/app/src/application` владеет состоянием и действиями desktop. Работает
  через `DaemonClient` без `egui`. `ui` отвечает за отрисовку, `desktop` — за
  жизненный цикл окна и tray.
- `decky-plugin` содержит TypeScript-панель и Python-прокси. `decky-bridge`
  содержит Rust-типы. `manifest-gen` — инструмент генерации манифестов.

Unit-тесты хранятся в `src/tests/` соответствующего crate и сгруппированы по
подсистемам. Публичные сценарии идут в `tests/`. Сквозная проверка бинарника
демона — `scripts/smoke.py`.
