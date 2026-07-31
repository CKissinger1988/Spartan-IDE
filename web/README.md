# Spartan Web (vscode.dev-inspired browser IDE)

Real, browser IDE for Spartan, built around the same **hybrid** architecture
decision the user made explicitly (§75.85, §75.86): editing/buffer logic
works standalone client-side in the browser (File System Access + WASM, no
backend needed), with a second, independent backend-mode editing path
activating whenever a local `spartan-devserver` instance is reachable over
its real WebSocket transport (§75.88). **When a devserver is connected: Git
(including PR listing, per-hunk staging, branches, stash, tags, merge/
cherry-pick), real LSP diagnostics/hover/autocomplete/rename/references,
real DAP breakpoint/step/watch debugging, real Android device/build/logcat
tooling, and a real, persistent Leo chat panel (task #285 — a direct port of
`desktop/`'s own) are all real and wired here.** See "What's not built yet"
below for the honest, current account of what's genuinely still missing.

Inspired by vscode.dev's *concepts* only — a browser-based editor working
directly against real local files via a native browser API, no server round
trip required for basic editing. **Zero VS Code or vscode.dev source code was
read or used anywhere in this project.** This is the same standing rule this
repository already applies to the desktop shells (no Monaco/CodeMirror

## Screenshots

Real, unedited Playwright + Chromium captures of the actual compiled
production build (`npm run build`) served by a real running
`spartan-devserver` process against a real git project fixture, using the
app's own genuine `BackendClient.connect()` — no mock, no shim of any kind.
These show the backend-connected mode (Git, real tree-sitter syntax
highlighting, Leo panel), the richer of `web/`'s two real editing paths and
the one most of this project's recent feature work has landed in; the
File System Access + WASM pure-client-side path (`components/Editor.tsx`,
`components/FileTree.tsx`) is exercised instead by the headless
File-System-Access-substitution technique described above under "Real,
executed verification".

| | |
|---|---|
| ![Initial connected state](../docs/screenshots/web/01-initial-empty-state.png) | ![Project opened, file tree](../docs/screenshots/web/02-project-opened-file-tree.png) |
| Connected to a local devserver — toolbar status, Leo panel, Backend file tree, no file open yet | The real Backend file tree expanded, showing a real project's directory structure |
| ![Editor with syntax highlighting](../docs/screenshots/web/03-editor-with-syntax-highlighting.png) | ![Live editing with dirty marker](../docs/screenshots/web/04-editing-live-highlight-dirty-marker.png) |
| Real tree-sitter syntax highlighting, opened through the Backend file tree | Live typing re-highlights immediately; the tab and status bar show the real unsaved marker |
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
- **Real undo and redo** via the real `Document`'s own branching undo tree,
  exposed through the WASM binding (Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y). Redo is
  a thin layer above `Document` (a `redo_stack`), the same pattern every
  other real Spartan UI surface already uses — closed in `spartan-buffer-
  wasm` itself, matching this crate's own doc comment.
- **Real client-side syntax highlighting** via a three-tier chain — real
  tree-sitter (`web-tree-sitter`, in-process WASM parsing) for the 8
  languages with a bundled grammar, `highlight.js` while a grammar is still
  loading and for languages with none, plain escaped text as the final
  fallback — the same chain, same capture-name-to-CSS mapping, and the same
  incremental (per-document, not whole-file-per-keystroke) re-parse
  `desktop/src/syntax.ts`/`treeSitter.ts` already use; this file is a direct
  copy of those two.
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

- **Git, LSP (diagnostics/hover/completion/rename/references/call
  hierarchy), DAP, Android tooling, and Leo's own chat panel are all real
  and wired here now** (this section used to say Leo's chat UI was the one
  real gap left in this app specifically — task #285 closed that; the
  original account below is kept since it's still the real story of *how*
  the backend-mode path came to exist, just corrected where it went stale).
  A later increment (Track A,
  `crates/spartan-devserver`) answered the
  token-delivery design question this section used to name as unresolved:
  the devserver serves this app's own static files, so a same-origin
  `fetch("/__spartan/session")` (see `backendClient.ts`) safely hands a
  connected page the live WebSocket token + Origin, and a further increment
  extended that same handoff to advertise the devserver's own real,
  canonicalized `--project-root:` as `projectRoot` — the piece that was
  still missing even after the transport question was solved, since
  `spartan-backend`'s `git_status`/`open_file`/Leo methods all need a real
  absolute filesystem path, and the File System Access API deliberately
  never exposes one for a folder picked via `showDirectoryPicker()` (a
  real, permanent browser security property, not an oversight). When a
  devserver is connected and advertises a project root, `App.tsx` shows a
  real Files/Git sidebar toggle and `components/GitPanel.tsx` — a direct
  port of `desktop/src/components/GitPanel.tsx` (§75.65) onto
  `BackendClient.call` — drives real `git_status`/`git_stage`/
  `git_unstage`/`git_commit` against that root. Real, live-verified:
  starting `spartan-devserver --project-root:<a real temp git repo>` and
  driving the served app with Playwright staged a real modified file,
  committed it, and the resulting commit was independently confirmed via
  `git log`/`git show` run directly against the repo on disk.
- **LSP is now real and wired here too, via a second, backend-mode editing
  path.** `spartan-backend`'s own `open_file`/`edit`/`undo`/`redo` spawn/
  drive a real language-server session and stream `lsp_diagnostics`/
  `lsp_error` events (closing a gap that had existed in *both*
  Electron-based shells since the pivot away from the wgpu reference
  shell) — `desktop/`'s Editor already rendered this live since its own
  file-open/edit path always went through the backend's IPC methods
  unconditionally, but this app's original editing path (File System
  Access + WASM) has no `doc_id` for that wiring to key off of. Closed by
  adding a real, independent second path: `components/BackendFileTree.tsx`
  (a direct port of `desktop/src/components/FileTree.tsx` onto
  `BackendClient.call`, rooted at the devserver's own project root) and
  `components/BackendEditor.tsx` (a direct port of `desktop/src/
  components/Editor.tsx`, same real edit/undo/redo/save/diagnostics
  wiring, reached over the WebSocket transport instead of Electron IPC).
  A third sidebar tab, "Backend", appears alongside Files/Git once a
  devserver with a known project root is connected; `App.tsx` tracks
  whichever of the two editing paths was opened most recently as one
  discriminated `activeContent` slot rather than two independent "current
  file" states. Real, live-verified against a real running devserver +
  `pyright-langserver`: opened a real file with a real deliberate type
  error via the Backend tab, confirmed the real diagnostic rendered in the
  gutter (screenshotted) matching `desktop/`'s own treatment exactly, typed
  a real live fix through the actual textarea, and confirmed the
  diagnostic genuinely cleared.
- **Real LSP hover and autocomplete were added in later increments, ported
  the same way.** `BackendEditor.tsx` gained the identical hover-tooltip
  (mouse-hold, pixel-to-line/character mapping) and Ctrl+Space completion
  dropdown logic `desktop/`'s own `Editor.tsx` already had, reached over
  the same `BackendClient` with zero backend/protocol changes needed (both
  methods were already generic `spartan-backend` methods). Real, live-
  verified against a real running devserver + `pyright-langserver`, same
  technique as the diagnostics verification above.
- **Real DAP (breakpoint/step debugging) was added too, in the same
  desktop-then-web sequencing.** Click-to-toggle gutter breakpoints and a
  compact `DebugPanel.tsx` (Debug/Continue/Step Over/Step Into/Stop, inline
  stack-frame/variable display) — a direct port of `desktop/`'s own
  `DebugPanel.tsx` — reached over the identical generic `BackendClient`
  call surface, no protocol changes needed. Real, live, end-to-end
  verified: a real Python fixture + a real `debugpy.adapter` session hit a
  breakpoint, rendered the correct stopped line/variable, and continued to
  a real exit, all through the actual compiled `web/dist` served by a real
  running `spartan-devserver` binary — no mock, unlike `desktop/`'s own
  verification (which needs a mocked `window.spartan` since the real
  Electron binary can't launch in this project's own sandboxed sessions;
  `web/` needs no such mock at all).
- **Real Android device/build/logcat tooling was added too.** A status-bar
  badge mirrors `desktop/`'s own: detects an Android/Gradle project, runs a
  real `gradle assembleDebug` build with streamed progress, lists real
  `adb` devices, installs the built APK, and streams real `adb logcat`
  output — all through the same generic `BackendClient` call surface (zero
  protocol changes needed for any of it, since every method is already a
  generic `spartan-backend` dispatch method). Real, live, end-to-end
  verified against a real Android/Gradle fixture and this environment's
  own real (if device-less) `adb`/`gradle` installs.
- **Leo's own chat panel is real and wired in (task #285).** `LeoChatPanel.tsx`
  is a direct port of `desktop/`'s own — same real state machine, same
  `leo_start_task`/`leo_approve_plan`/`leo_next_step`/`leo_approve_call`/
  `leo_retry`/`leo_cancel`/`leo_session_history` dispatch methods, same
  random-thoughts/voice-I/O features — reached over `BackendClient` with
  zero protocol changes (every method was already a generic
  `spartan-backend` dispatch method). Docked as a persistent sidebar
  whenever a devserver connection is live, matching `desktop/`'s own
  always-visible placement.
- **The two editing paths are independent, not unified.** A real, named
  consequence, not an oversight: the folder opened via "Open Folder…"
  (File System Access) and the devserver's own project root
  (`--project-root:`) can be different directories — nothing in this app
  verifies they match. In the common case (the devserver is launched from
  the same project the user opens), the two happen to agree, but this app
  makes no attempt to enforce or check that.
- **Multi-file tabs are real (task #257)**, covering both editing paths
  combined — `openTabs`/`activeIndex` in `App.tsx` hold every open file (of
  either kind) as one derived-state tab model, reusing an already-open tab
  of the same kind+path rather than duplicating it, matching `desktop/`'s
  own tab-bar convention.
- **Chromium-only.** The File System Access API is not implemented in
  Firefox or Safari. This is a real, permanent platform limit, not a bug —
  the app detects this and shows an honest message instead of failing
  silently or crashing.

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
- `src/backendClient.ts` — the real `BackendClient`: session handoff +
  WebSocket connection to a local `spartan-devserver`, exposing its
  advertised `projectRoot` alongside the usual `call`/`onEvent` surface.
- `src/components/GitPanel.tsx` — real Source Control panel, a direct port
  of `desktop/src/components/GitPanel.tsx` onto `BackendClient.call`, shown
  only once a devserver connection advertises a real project root.
- `src/components/BackendFileTree.tsx`, `src/components/BackendEditor.tsx`
  — the real backend-mode editing path, direct ports of `desktop/src/
  components/FileTree.tsx`/`Editor.tsx` onto `BackendClient.call`, shown
  under a "Backend" sidebar tab alongside Files/Git once a devserver
  connection advertises a real project root. This is what makes real,
  live LSP diagnostics (`lsp_diagnostics`/`lsp_error` events) usable in
  this app -- the File System Access + WASM path (`FileTree.tsx`/
  `Editor.tsx`) has no `doc_id` for that wiring to key off of.
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
