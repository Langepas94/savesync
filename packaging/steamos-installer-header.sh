#!/bin/bash
# Self-extracting installer; the release archive follows the marker below.
set -uo pipefail
if ! command -v kdialog >/dev/null 2>&1; then
  echo 'Open this installer in Steam Deck Desktop Mode (KDE required).' >&2
  exit 1
fi
kdialog --yesno 'Установить SaveSync для текущего пользователя?' --title 'Установка SaveSync' || exit 0
stage=$(mktemp -d "${TMPDIR:-/tmp}/savesync-installer.XXXXXXXX") || exit 1
trap 'rm -rf -- "$stage"' EXIT
log="$HOME/savesync-install.log"
marker=$(awk '/^__SAVESYNC_ARCHIVE_BELOW__$/ { print NR + 1; exit }' "$0")
if [[ -n "$marker" ]] && tail -n +"$marker" "$0" | tar -xz -C "$stage" >"$log" 2>&1 &&
   bash "$stage/savesync-steamos/install-steamos.sh" >>"$log" 2>&1; then
  kdialog --msgbox 'SaveSync установлен. Откройте его через меню приложений.' --title 'SaveSync'
else
  kdialog --error "Не удалось установить SaveSync. Подробности: $log" --title 'SaveSync'
  exit 1
fi
exit 0
__SAVESYNC_ARCHIVE_BELOW__
