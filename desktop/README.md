# Spartan Desktop (Electron shell)

Real Electron + React frontend for Spartan IDE, driving the existing Rust
core (`spartan-buffer`, and eventually tree-sitter/LSP/DAP/Leo/`spartan-git`)
over a local IPC service (`crates/spartan-backend`) instead of the original
wgpu-native renderer in `crates/spartan-editor-core`. See
`docs/architecture-spec.md` §75.59 for why this exists alongside that crate
(kept as the real, tested backend proof-of-concept and reference, not
deleted).

## Layout

- `electron/main.ts` — main process: creates the `BrowserWindow`, spawns the
  real `spartan-backend` release binary as a child process, registers the six
  `spartan:*` IPC channels.
- `electron/preload.ts` — the one real, narrow `contextBridge` surface the
  renderer gets (`window.spartan.call(method, params)`); no raw `ipcRenderer`,
  no Node API, `nodeIntegration: false` + `contextIsolation: true`.
- `electron/backend-client.ts` — spawns `spartan-backend` and speaks its real
  newline-delimited JSON protocol over stdin/stdout.
- `src/` — the React renderer: `FileTree`, `TabBar`, `ModeToggle`,
  `StatusBar`, and `Editor` (a real, custom -- not Monaco/CodeMirror --
  text-editing surface; see `Editor.tsx`'s own doc comment for exactly what
  "custom" means here and what it doesn't yet do).

## Build & run

```bash
# 1. Build the real Rust backend (from the repo root):
cargo build --release -p spartan-backend

# 2. Install desktop deps:
cd desktop
npm install

# 3. Dev mode (two terminals):
npm run dev:renderer     # Vite dev server on :5173
npm run build:electron   # compiles electron/*.ts once, then:
electron .                # (or `npm run start` after both are built)
```

## A real, environment-specific gap in the session that built this

`npm install` in the session that wrote this code could not complete a
normal install: `electron`'s postinstall script downloads the actual
Electron runtime binary from `github.com/electron/electron/releases/...`,
and that host is blocked (403) by that session's own egress policy — a
real, reported, not-routed-around limitation (no mirror substitution was
attempted), not a bug in this code. `ELECTRON_SKIP_BINARY_DOWNLOAD=1 npm
install` was used instead to get every other real dependency installed and
both `tsc` projects (`tsconfig.json`, `electron/tsconfig.json`) type-checking
clean, and the real React renderer was verified live via a Vite dev server
plus the environment's own pre-installed Playwright Chromium (with a
test-only `window.spartan` stub standing in for Electron's real preload
bridge, never shipped) — see §75.59 for the full, honest account of what
that did and didn't confirm. **The actual Electron window/native chrome and
the real IPC wiring through a genuine Electron process have not been
launched or screenshotted in that session** — that needs a real `npm
install` (no `ELECTRON_SKIP_BINARY_DOWNLOAD`) run somewhere with access to
GitHub releases, then `npm run start`.
