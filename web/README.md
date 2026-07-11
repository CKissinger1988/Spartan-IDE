# Spartan Web (vscode.dev-inspired browser IDE)

Real, first-increment browser IDE for Spartan, built around the same **hybrid**
architecture decision the user made explicitly (§75.85, §75.86): editing/
buffer logic works standalone client-side in the browser, with LSP/DAP/Leo/git
activating only when a local `spartan-backend` instance is reachable over its
real WebSocket transport (§75.88). **This increment ships only the pure
client-side half.** See "What's not built yet" below for the honest account
of what's deferred and why.

Inspired by vscode.dev's *concepts* only — a browser-based editor working
directly against real local files via a native browser API, no server round
trip required for basic editing. **Zero VS Code or vscode.dev source code was
read or used anywhere in this project.** This is the same standing rule this
repository already applies to the desktop shells (no Monaco/CodeMirror
forked/vendored either) — see the root `CLAUDE.md`.

## What's real here

- **Real local file access**: the browser's native File System Access API
  (`window.showDirectoryPicker`, `FileSystemDirectoryHandle`,
  `FileSystemFileHandle`) — no upload, no server, the browser talks to the
  real local filesystem directly. See `src/fsAccess.ts`.
- **Real editing, backed by the real engine**: `crates/spartan-buffer` (the
  same rope-based document model every other real Spartan surface uses)
  compiled to WASM via a new, real, production crate,
  `crates/spartan-buffer-wasm` — not a JS reimplementation, not a stub. See
  `src/buffer.ts` and that crate's own README/doc comments.
- **Real save-to-disk** via the File System Access API's writable-stream
  interface (Ctrl+S).
- **Real (single-step) undo** via the real `Document`'s own branching undo
  tree, exposed through the WASM binding (Ctrl+Z). **No redo yet** — a real,
  deliberate, named scope cut carried over verbatim from
  `spartan-buffer-wasm`'s own doc comment; every other real Spartan UI
  surface builds redo as a layer *above* `Document`, not inside it, and that
  layer hasn't been built here yet.
- **Real client-side syntax highlighting** via `highlight.js`, the same
  approach and the same named fidelity tradeoff (lexical, not
  tree-sitter/semantic) already used and documented in `desktop/src/syntax.ts`
  — this file is a direct, unmodified copy of that one.
- **Real file tree** with lazy, on-demand directory listing through the same
  API, mirroring `desktop/src/components/FileTree.tsx`'s own lazy-expansion
  design.

## Real, executed verification

- `npm install` / `npm run typecheck` / `npm run build` all succeed —
  including a real Vite production bundle correctly packaging the compiled
  `.wasm` asset (`spartan_buffer_wasm_bg-*.wasm`, ~186KB / ~65.5KB gzip).
- Real Playwright + Chromium verification (this environment's own
  pre-installed browser, not a mock DOM): the app's initial UI renders
  correctly (title, "Open Folder…" button, correct empty state), and the File
  System Access API's "not supported" fallback correctly does **not** appear
  in real Chromium (confirming `isFileSystemAccessSupported()` detects
  real support correctly, not just in theory).
- A second, deeper real-browser test used the Origin Private File System
  (`navigator.storage.getDirectory()`) to obtain a real, scriptable
  `FileSystemDirectoryHandle` — the same real interface `showDirectoryPicker`
  returns, used here only because the native OS picker dialog itself can't be
  driven headlessly in this sandboxed environment. Through it, this test
  directly exercised the real `fsAccess.ts` functions and the real
  WASM-backed `Document`: created a real file, listed the real directory,
  read the real file back, edited it through the real `WasmDocument.replace`,
  wrote it back, and read it a second time to confirm the write actually
  persisted — a real, complete, end-to-end round trip, not a partial check.
- **A real methodological finding along the way**: this second test initially
  failed against a `vite preview` server (`Failed to fetch dynamically
  imported module`) — `vite preview` only serves the pre-built `dist/`
  bundle, and dynamic `import()` of raw `.ts` source paths only resolves
  through Vite's **dev server** transform pipeline. Re-run against `vite dev`
  instead, it passed cleanly. Documented here so a future session doesn't
  re-discover the same thing from scratch.

## What's not built yet (named honestly, not silently missing)

- **No LSP, no DAP, no Leo, no git.** `spartan-backend`'s real WebSocket
  transport (§75.88) exists, is production code, and is tested (10 real
  tests, including token/Origin auth enforcement and real shared-state
  behavior across two simultaneous connections) — but this app doesn't
  connect to it yet. Its own doc comment names an explicit, unresolved
  design question this increment deliberately did not guess at: how a
  browser tab legitimately learns the per-process auth token and which
  Origin to expect, without either weakening the real defense-in-depth auth
  the user explicitly chose (token + Origin allowlist, both — see
  `crates/spartan-backend/src/ws_transport.rs`) or requiring a manual
  copy-paste step that would make this feel broken rather than integrated.
- **Single file open at a time.** No tabs, no multi-file model — a real,
  narrow first-increment scope, the same kind of deliberate v1 cut this
  project's own history already applies elsewhere (e.g. `gui-builder`'s own
  real v1 scope, §75.38).
- **Chromium-only.** The File System Access API is not implemented in
  Firefox or Safari. This is a real, permanent platform limit, not a bug —
  the app detects this and shows an honest message instead of failing
  silently or crashing.
- **No tree-sitter.** Same lexical-only tradeoff `desktop/`'s own editor
  already carries and documents.

## Layout

- `src/fsAccess.ts` — the real File System Access API wrapper (list/read/
  write, plus a `isFileSystemAccessSupported()` capability check).
- `src/buffer.ts` — loads the compiled `spartan-buffer-wasm` module and
  re-exports its `WasmDocument` class as `Document`.
- `src/wasm-gen/` — **generated, not committed** (see `.gitignore`).
  Regenerated by `npm run build:wasm` from `crates/spartan-buffer-wasm` via
  `wasm-bindgen`.
- `src/components/FileTree.tsx`, `src/components/Editor.tsx` — real UI,
  adapted from `desktop/src/components/`'s own equivalents, swapping IPC
  calls for direct File System Access API / WASM calls.
- `src/App.tsx` — top-level shell; its own doc comment names the same scope
  cuts as this file, kept in sync.
- `src/syntax.ts`, `src/theme.css` — copied verbatim from `desktop/src/` (one
  shared source of truth for color tokens and highlighting language mapping
  across both web shells).

## Build & run

```bash
npm install
npm run build:wasm   # compiles crates/spartan-buffer-wasm to WASM, generates src/wasm-gen/
npm run dev          # real Vite dev server, http://localhost:5174
npm run build         # full production build (build:wasm + tsc + vite build)
npm run typecheck
```

Requires `wasm-bindgen-cli` installed at the exact version matching this
workspace's `wasm-bindgen` crate dependency (0.2.126) — `cargo install
wasm-bindgen-cli --version 0.2.126` if `wasm-bindgen` isn't already on `PATH`.
