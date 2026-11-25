#!/bin/bash
# User-only install/update, with staging and rollback. Never changes SteamOS rootfs.
# ./install-steamos.sh [path/to/savesync-daemon] [--no-start]
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "$0")" && pwd -P)"
source_daemon=''
start_service=1
for argument in "$@"; do
  if [[ "$argument" == '--no-start' ]]; then start_service=0
  elif [[ -z "$source_daemon" ]]; then source_daemon="$argument"
  else echo 'usage: install-steamos.sh [daemon-path] [--no-start]' >&2; exit 2; fi
done
if [[ -z "$source_daemon" ]]; then
  for candidate in "$script_dir/savesync-daemon" "$script_dir/../target/release/savesync-daemon"; do
    if [[ -x "$candidate" ]]; then source_daemon="$candidate"; break; fi
  done
fi
if [[ -z "$source_daemon" || ! -x "$source_daemon" ]]; then echo 'daemon executable missing' >&2; exit 1; fi
source_dir="$(cd -- "$(dirname -- "$source_daemon")" && pwd -P)"
source_daemon="$source_dir/savesync-daemon"
source_app="$source_dir/savesync-app"
if [[ ! -x "$source_app" || ! -f "$script_dir/systemd/savesync.service" ]]; then echo 'complete app/daemon package and systemd unit required' >&2; exit 1; fi
daemon_version="$("$source_daemon" --version)"
app_version="$("$source_app" --version)"
if [[ "${daemon_version##* }" != "${app_version##* }" ]]; then echo 'app and daemon versions differ' >&2; exit 1; fi
home_dir="$(cd -- "$HOME" && pwd -P)"
mkdir -p "$home_dir/.local/bin" "$home_dir/.local/share/savesync" "$home_dir/.config/systemd/user" "$home_dir/.local/share/applications"
for directory in "$home_dir/.local/bin" "$home_dir/.local/share/savesync" "$home_dir/.config/systemd/user" "$home_dir/.local/share/applications"; do
  resolved="$(cd -- "$directory" && pwd -P)"
  case "$resolved/" in "$home_dir/"*) ;; *) echo 'installation target must remain inside home' >&2; exit 1;; esac
done
exec 9>"$home_dir/.local/share/savesync/install.lock"
flock -n 9 || { echo 'another installation is running' >&2; exit 1; }
stage="$(mktemp -d "$home_dir/.local/bin/.savesync-stage.XXXXXXXX")"
targets=("$home_dir/.local/bin/savesync-daemon" "$home_dir/.local/bin/savesync-app" "$home_dir/.config/systemd/user/savesync.service" "$home_dir/.local/share/applications/savesync.desktop")
changed=0
was_running=0
was_enabled=0
stopped=0
finish() {
  result=$?
  trap - EXIT
  if [[ "$result" != 0 && "$changed" == 1 ]]; then
    if [[ "$start_service" == 1 ]]; then
      "${targets[0]}" stop || true
      systemctl --user stop savesync.service || true
      if [[ "$was_enabled" == 0 ]]; then systemctl --user disable savesync.service || true; fi
    fi
    for number in "${!targets[@]}"; do
      if [[ -f "$stage/old-$number" ]]; then cp -p -- "$stage/old-$number" "${targets[$number]}"
      else rm -f -- "${targets[$number]}"; fi
    done
    if [[ "$start_service" == 1 ]]; then
      systemctl --user daemon-reload || true
    fi
  fi
  if [[ "$result" != 0 && "$stopped" == 1 && "$was_running" == 1 && "$start_service" == 1 ]]; then
    systemctl --user start savesync.service || true
  fi
  case "$stage" in "$home_dir/.local/bin/.savesync-stage."*) rm -rf -- "$stage";; *) echo 'unsafe staging cleanup path' >&2; exit 1;; esac
  exit "$result"
}
trap finish EXIT
for target in "${targets[@]}"; do if [[ -L "$target" ]]; then echo 'installation targets must not be symlinks' >&2; exit 1; fi; done
for number in "${!targets[@]}"; do if [[ -e "${targets[$number]}" ]]; then cp -p -- "${targets[$number]}" "$stage/old-$number"; fi; done
install -m 0755 "$source_daemon" "$stage/new-0"
install -m 0755 "$source_app" "$stage/new-1"
install -m 0644 "$script_dir/systemd/savesync.service" "$stage/new-2"
desktop_exec="${targets[1]//\\/\\\\}"
desktop_exec="${desktop_exec//\"/\\\"}"
printf '[Desktop Entry]\nType=Application\nName=SaveSync\nExec="%s"\nIcon=folder-sync\nTerminal=false\nCategories=Game;Utility;\n' "$desktop_exec" > "$stage/new-3"
chmod 0644 "$stage/new-3"
if [[ "$start_service" == 1 ]]; then
  if systemctl --user is-active --quiet savesync.service; then was_running=1; fi
  if systemctl --user is-enabled --quiet savesync.service; then was_enabled=1; fi
  stopped=1
  if [[ -x "${targets[0]}" ]]; then "${targets[0]}" stop; fi
  if [[ -x "${targets[1]}" ]]; then "${targets[1]}" --quit; fi
  if [[ -f "${targets[2]}" ]]; then systemctl --user stop savesync.service; fi
fi
changed=1
for number in "${!targets[@]}"; do mv -f -- "$stage/new-$number" "${targets[$number]}"; done
if [[ "$start_service" == 1 ]]; then
  systemctl --user daemon-reload
  systemctl --user enable --now savesync.service
  ready=0
  for attempt in {1..300}; do
    if "${targets[0]}" status >/dev/null 2>&1; then ready=1; break; fi
    sleep 0.1
  done
  if [[ "$ready" == 0 ]]; then echo 'daemon did not become ready; restoring previous package' >&2; exit 1; fi
fi
echo "SaveSync installed: ${targets[0]}"
echo 'Profile and credentials preserved. Open SaveSync in Desktop Mode to configure OAuth/LAN.'
