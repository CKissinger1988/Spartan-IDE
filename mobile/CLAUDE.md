@AGENTS.md

# Spartan Mobile IDE

Companion mobile app for Spartan IDE — this `mobile/` subdirectory, same repo and branch as the
desktop app, not a separate repo (it briefly was one; moved in via `git subtree` with its commit
history preserved intact). Read `README.md` first: what this app is/isn't, what's real vs. mock
data, and what hasn't been run (no device/emulator available when this was built).

Design source of truth lives one level up: `../docs/architecture-spec.md` §69 (§69.1–§69.6).
Read that before adding scope here — v1's boundaries (no code editing, no debugger, no terminal,
no Developer Mode) are deliberate, not gaps.

## Rules

- Same discipline as the desktop project: don't claim something works without running it.
  `npx tsc --noEmit` and `npx expo export --platform android` are the two checks that currently
  stand in for real device testing — run both after any change, and say so explicitly if a
  change can't be verified further without a device/emulator.
- Mock data (`src/data/mockData.ts`) stands in for a session-store backend that doesn't exist
  yet. Don't wire up fake "success" behavior that implies a real backend call happened.
