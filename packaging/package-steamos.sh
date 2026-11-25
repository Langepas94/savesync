#!/bin/bash
# Package only completed binaries/frontend, preserving Unix executable modes.
set -euo pipefail
repo="$(cd -- "$(dirname -- "$0")/.." && pwd -P)"
source_dir="$(cd -- "${1:-$repo/target/release}" && pwd -P)"
output="${2:-$repo/dist}"
for binary in savesync-daemon savesync-app; do test -x "$source_dir/$binary" || { echo "missing $binary" >&2; exit 1; }; done
test -f "$repo/decky-plugin/dist/index.js" || { echo 'build the Decky frontend first' >&2; exit 1; }
daemon_version="$("$source_dir/savesync-daemon" --version)"
app_version="$("$source_dir/savesync-app" --version)"
test "${daemon_version##* }" = "${app_version##* }" || { echo 'binary versions differ' >&2; exit 1; }
mkdir -p "$output"
output="$(cd -- "$output" && pwd -P)"
stage="$(mktemp -d "$output/.savesync-package.XXXXXXXX")"
cleanup() { case "$stage" in "$output"/.savesync-package.*) rm -rf -- "$stage";; *) exit 1;; esac; }
trap cleanup EXIT
package="$stage/savesync-steamos"
mkdir -p "$package/systemd" "$package/decky-plugin/dist" "$package/docs" "$package/licenses"
install -m 0755 "$source_dir/savesync-daemon" "$source_dir/savesync-app" "$package/"
install -m 0755 "$repo/packaging/install-steamos.sh" "$repo/packaging/verify-steamos.sh" "$package/"
install -m 0644 "$repo/packaging/systemd/savesync.service" "$package/systemd/"
install -m 0644 "$repo/decky-plugin/main.py" "$repo/decky-plugin/plugin.json" "$repo/decky-plugin/package.json" "$package/decky-plugin/"
cp -r -- "$repo/decky-plugin/dist/." "$package/decky-plugin/dist/"
install -m 0644 "$repo/docs/OAUTH.md" "$repo/docs/LAN.md" "$repo/docs/INDEX-PUBLICATIONS.md" "$package/docs/"
sed -e 's#](OAUTH.md)#](docs/OAUTH.md)#g' -e 's#](LAN.md)#](docs/LAN.md)#g' "$repo/docs/STEAMOS-DELIVERY.md" > "$package/README.md"
install -m 0644 "$repo/crates/core/manifests/LICENSE" "$package/licenses/manifests.txt"
printf '%s\n' "${daemon_version##* }" > "$package/VERSION"
tar -czf "$output/savesync-steamos.tar.gz.tmp" -C "$stage" savesync-steamos
mv -f -- "$output/savesync-steamos.tar.gz.tmp" "$output/savesync-steamos.tar.gz"
(cd -- "$output" && sha256sum savesync-steamos.tar.gz > savesync-steamos.tar.gz.sha256)
echo "Built $output/savesync-steamos.tar.gz"
