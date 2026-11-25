#!/bin/bash
# Read-only checks for a real Deck. Does not sync, restore, install or change rootfs.
set -u
failures=0
check() { if "$@"; then echo "PASS: $*"; else echo "FAIL: $*" >&2; failures=$((failures+1)); fi; }
check "$HOME/.local/bin/savesync-daemon" --version
check "$HOME/.local/bin/savesync-app" --version
check systemctl --user is-active savesync.service
check systemctl --user is-enabled savesync.service
check "$HOME/.local/bin/savesync-daemon" status
check test -f "$HOME/.local/share/applications/savesync.desktop"
if command -v steamos-readonly >/dev/null 2>&1; then check steamos-readonly status; else echo 'INFO: steamos-readonly is absent; this is not a confirmed SteamOS host'; fi
if command -v ldd >/dev/null 2>&1; then
  for binary in "$HOME/.local/bin/savesync-daemon" "$HOME/.local/bin/savesync-app"; do
    dependencies="$(ldd "$binary" 2>&1)"
    if [[ "$dependencies" == *'not found'* ]]; then echo "$dependencies"; failures=$((failures+1)); fi
  done
fi
if [[ -n "${SAVESYNC_VAULT_PASSPHRASE:-}" ]]; then echo 'INFO: encrypted credential vault selected (passphrase not printed)'
elif [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then echo 'INFO: D-Bus session present; Secret Service unlock must be verified during real OAuth'
else echo 'INFO: configure Secret Service or the encrypted vault before cloud login'; fi
echo 'Physical gate still requires: real OAuth consent/refresh, LAN pairing across two hosts, Decky QAM display and a disposable save roundtrip.'
exit "$failures"
