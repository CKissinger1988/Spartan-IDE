#!/usr/bin/env bash
set -euo pipefail

printf '\033[36m[Spartan]\033[0m Installing Agent Deck from upstream...\n'
curl -fsSL https://raw.githubusercontent.com/asheshgoplani/agent-deck/main/install.sh | bash -s -- --non-interactive
printf '\033[32m[Spartan]\033[0m Agent Deck install script finished. Run: ./bin/spartan-agent-deck doctor\n'
