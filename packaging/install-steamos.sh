#!/bin/bash
# SaveSync — установка на Steam Deck (SteamOS 3.x, Desktop Mode).
# Только home-каталог: без sudo, без steamos-readonly disable.
#
#   ./install-steamos.sh [путь-к-savesync-daemon]
#
# По умолчанию бинарь берётся рядом со скриптом или из target/release.
set -euo pipefail

BIN_SRC="${1:-}"
if [[ -z "$BIN_SRC" ]]; then
  for candidate in ./savesync-daemon ../target/release/savesync-daemon; do
    if [[ -x "$candidate" ]]; then BIN_SRC="$candidate"; break; fi
  done
fi
if [[ -z "$BIN_SRC" || ! -x "$BIN_SRC" ]]; then
  echo "ошибка: не найден исполняемый savesync-daemon" >&2
  echo "укажите путь: ./install-steamos.sh /path/to/savesync-daemon" >&2
  exit 1
fi

mkdir -p "$HOME/.local/bin" "$HOME/.config/systemd/user"
install -m 0755 "$BIN_SRC" "$HOME/.local/bin/savesync-daemon"
install -m 0644 "$(dirname "$0")/systemd/savesync.service" \
  "$HOME/.config/systemd/user/savesync.service"

# ~/.local/bin должен быть в PATH (в SteamOS он есть; проверим на всякий).
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo "note: добавьте $HOME/.local/bin в PATH (например, в ~/.bashrc)";;
esac

systemctl --user daemon-reload
systemctl --user enable --now savesync.service

echo "✓ SaveSync установлен: $HOME/.local/bin/savesync-daemon"
echo "✓ Сервис: systemctl --user status savesync"
echo
echo "Дальше: настройте канал синхронизации:"
echo "  savesync-daemon remote setup fs /путь/к/папке      # USB/локальная папка"
echo "  savesync-daemon remote setup yandex <OAuth-токен>  # Яндекс.Диск"
echo "  savesync-daemon remote setup google <token>        # Google Drive"
echo "  savesync-daemon scan && savesync-daemon sync       # первый прогон"
