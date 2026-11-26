# Agent tools

## AST Index

Installed CLI: `@ast-index/cli` 3.56.0. Requires Node.js 14 or newer.

```powershell
npm.cmd install -g @ast-index/cli@3.56.0
ast-index rebuild
ast-index stats
ast-index search Manifest
ast-index symbol Manifest
ast-index refs Manifest
ast-index update
```

Run these commands from `savesync/`. The enclosing workspace also has a configuration and its own index for commands run from that directory. `.ast-index.yaml` includes Rust crates, the Decky plugin, and packaging scripts, excluding dependencies and generated output. SQLite databases live in the user cache outside Git. Run `ast-index update` after edits or branch changes, or keep `ast-index watch` running in a separate terminal. Rebuild after changing the configuration.

The CLI is the configured integration; an MCP server is not required or registered.

## Caveman

The project-local skill is vendored in `.agents/skills/caveman/` from `JuliusBrussee/caveman`, tag `v4.2.0`. `AGENTS.md` enables it by default. It shortens replies while preserving the user's language, code, commands, paths, errors, and technical meaning. Use `$caveman` to request it explicitly; `stop caveman` or `normal mode` disables it for the session. The installed skill becomes available on the next turn.

The companion `@caveman-ai/cli` 4.2.0 is installed globally and requires Node.js 22.13 or newer:

```powershell
npm.cmd install -g @caveman-ai/cli@4.2.0
caveman.cmd --help
```

This project uses the voice skill. The CLI's optional proxy, compression runtime, account integration, automatic model routing, and global agent hooks are not configured. `caveman status` reports those optional runtime components separately from the project skill.

Upstream: [AST Index](https://github.com/defendend/Claude-ast-index-search), [Caveman](https://github.com/JuliusBrussee/caveman).
