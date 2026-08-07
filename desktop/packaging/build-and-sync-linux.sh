#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
bundle_dir=${1:-"${HOME}/.local/opt/spartan-ide-linux-x64-05649f6"}

(cd "$repo_root/gui-builder" && npm run build)
(cd "$repo_root/desktop" && npm run build)
(cd "$repo_root" && cargo build --release -p spartan-backend)
"$repo_root/desktop/packaging/sync-linux-bundle.sh" "$bundle_dir"
