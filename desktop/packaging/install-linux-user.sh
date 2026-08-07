#!/usr/bin/env bash
set -euo pipefail

bundle_dir=${1:?usage: install-linux-user.sh <linux-bundle-directory>}
if [[ ! -x "$bundle_dir/electron" || ! -f "$bundle_dir/resources/app/package.json" ]]; then
  echo "Not a Spartan Linux bundle: $bundle_dir" >&2
  exit 1
fi

bundle_dir=$(cd "$bundle_dir" && pwd)
install_root=${SPARTAN_INSTALL_ROOT:-"$HOME/.local/opt"}
version_name=$(basename "$bundle_dir")
install_dir="$install_root/$version_name"

mkdir -p "$install_dir" "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/512x512/apps"
cp -a "$bundle_dir"/. "$install_dir"/
cp "$(dirname "$0")/spartan-ide-launcher" "$install_dir/spartan-ide"
chmod 755 "$install_dir/spartan-ide"
ln -sfn "$install_dir/spartan-ide" "$HOME/.local/bin/spartan-ide"
cp "$(dirname "$0")/spartan-ide.desktop" "$HOME/.local/share/applications/spartan-ide.desktop"
cp "$install_dir/resources/app/dist/assets/spartan-logo-By3Yr9vN.png" "$HOME/.local/share/icons/hicolor/512x512/apps/spartan-ide.png"

echo "Installed Spartan IDE to $install_dir"
echo "Launcher: $HOME/.local/bin/spartan-ide"
