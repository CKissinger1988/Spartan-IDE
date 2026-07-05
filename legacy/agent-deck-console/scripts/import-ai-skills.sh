#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REGISTRY="$ROOT_DIR/config/ai-skills.tsv"
POOL_DIR="${AGENT_DECK_SKILL_POOL:-$HOME/.agent-deck/skills/pool}"

SOURCES=(
  "/mnt/c/Users/ckiss/.antigravitycli/skills:antigravity"
  "/mnt/c/Users/ckiss/.codex/skills:codex"
)

mkdir -p "$POOL_DIR"

tmp="$(mktemp)"
printf "# id\tname\tsource\tpath\ttarget\tstatus\n" > "$tmp"

count=0
linked=0
missing=0

slugify() {
  printf "%s" "$1" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9._-]+/-/g; s/^-+//; s/-+$//'
}

import_skill() {
  local skill_file="$1"
  local source_name="$2"
  local skill_dir
  skill_dir="$(dirname "$skill_file")"
  local name
  name="$(basename "$skill_dir")"
  local slug
  slug="$(slugify "$source_name-$name")"
  local target="$POOL_DIR/$slug"

  count=$((count + 1))
  if [[ -e "$target" && ! -L "$target" ]]; then
    printf "%s\t%s\t%s\t%s\t%s\t%s\n" "$slug" "$name" "$source_name" "$skill_dir" "$target" "blocked-existing-directory" >> "$tmp"
    return
  fi

  ln -sfn "$skill_dir" "$target"
  linked=$((linked + 1))
  printf "%s\t%s\t%s\t%s\t%s\t%s\n" "$slug" "$name" "$source_name" "$skill_dir" "$target" "linked" >> "$tmp"
}

for source_spec in "${SOURCES[@]}"; do
  source_root="${source_spec%%:*}"
  source_name="${source_spec##*:}"
  if [[ ! -d "$source_root" ]]; then
    missing=$((missing + 1))
    continue
  fi
  while IFS= read -r skill_file; do
    import_skill "$skill_file" "$source_name"
  done < <(find "$source_root" -type f -name SKILL.md | sort)
done

mv "$tmp" "$REGISTRY"

printf "Spartan skill import complete\n"
printf "  discovered: %d\n" "$count"
printf "  linked:     %d\n" "$linked"
printf "  pool:       %s\n" "$POOL_DIR"
printf "  registry:   %s\n" "$REGISTRY"
if [[ "$missing" -gt 0 ]]; then
  printf "  missing source roots: %d\n" "$missing"
fi
