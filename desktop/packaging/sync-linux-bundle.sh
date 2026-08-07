#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
bundle_dir=${1:-"${HOME}/.local/opt/spartan-ide-linux-x64-05649f6"}
app_dir="$bundle_dir/resources/app"

if [[ ! -x "$bundle_dir/electron" || ! -d "$app_dir" ]]; then
  echo "Not a Spartan Linux bundle: $bundle_dir" >&2
  exit 1
fi

cp -a "$repo_root/desktop/dist/." "$app_dir/dist/"
cp -a "$repo_root/desktop/dist-electron/." "$app_dir/dist-electron/"
mkdir -p "$app_dir/gui-builder/dist" "$app_dir/gui-builder/node_modules"
cp -a "$repo_root/gui-builder/dist/." "$app_dir/gui-builder/dist/"
cp -a "$repo_root/gui-builder/node_modules/." "$app_dir/gui-builder/node_modules/"
backend_tmp="$bundle_dir/resources/.spartan-backend.sync"
cp "$repo_root/target/release/spartan-backend" "$backend_tmp"
chmod 755 "$backend_tmp"
mv -f "$backend_tmp" "$bundle_dir/resources/spartan-backend"

echo "Synchronized Linux bundle: $bundle_dir"
