# Spartan IDE — Future Features Backlog

This is a curated, prioritized list of **useful features to add later**. Every item here is a
*real, already-named gap* — drawn from the "What this does not confirm" / deferred-scope notes
that each implemented feature in `CLAUDE.md` records honestly, plus the roadmap tiers in
`docs/architecture-spec.md` (§35). Nothing here is speculative marketing; each is a concrete,
grounded next step against the code that already exists.

It is **not** a list of things that are broken. The shipped feature set is production-verified
(builds green across every project, security surfaces confirmed live, editor/git/LSP/DAP flows
exercised end-to-end). These are additive capabilities the architecture is already shaped to
accept.

Priority key:
- **P1** — high user value, near-term, builds directly on existing infrastructure.
- **P2** — meaningful value, moderate effort, or depends on a P1 first.
- **P3** — larger initiatives, infrastructure, or blocked on an external prerequisite
  (real hardware, a paid service, a network capability this dev environment lacks).

---

## Recommended next 10 (the highest value-to-effort items)

1. **Code folding** in the editor (P1) — ⚠️ **architecturally blocked** by the current
   `<textarea>`-backed editor: a textarea renders all its content and can't hide lines. A real
   implementation needs a from-scratch canvas/DOM-line editor (a large, separate initiative), or a
   hacky "remove folded text from the value, store it aside" scheme that fights every other edit
   operation. Deferred honestly, not "pure client-side, easy."
2. **LSP code actions / quick fixes** (P1) — the `codeAction` capability; wire the lightbulb UI.
   Real servers (rust-analyzer, typescript-language-server) implement it richly, **but this dev
   env only has pyright, which returns empty code-actions**, so it can't be strongly live-verified
   here — build it when a richer server is installable.
3. **Multi-cursor / multi-selection editing** (P1) — ⚠️ **architecturally blocked** by the
   `<textarea>` (native single caret/selection only); needs the same from-scratch editing-surface
   rewrite as code folding.
4. ~~**Git: remote push/pull/fetch + clone** (P1)~~ — ✅ **Shipped** (fetch/pull/push in both
   shells' Git panels; `spartan_git` remote ops against real remotes). Clone + auth-token UI
   remain follow-ups. See CLAUDE.md's own status entry.
5. ~~**Inline git blame** (P1)~~ — ✅ **Shipped** (Alt+B blame gutter in both shells, backed by
   `spartan_git::blame_file`). See CLAUDE.md's own status entry.
6. ~~**Snippets / tab-completion expansion** (P1)~~ — ✅ **Shipped** (curated per-language
   snippets, prefix+Tab expansion, tab-stop navigation, in all three editing surfaces). See
   CLAUDE.md's own status entry.
7. **tree-sitter syntax highlighting in the Electron shells** (P2) — replace the current
   `highlight.js` lexical pass with the real tree-sitter engine already used in the wgpu shell
   (via `web-tree-sitter`), for correctness parity.
8. ~~**Conditional breakpoints + logpoints** (P2)~~ — ✅ **done**. `spartan_dap::Breakpoint`
   carries `condition`/`log_message`, threaded through `dap_launch`'s `breakpoints:
   [{line, condition?, logMessage?}]` param (with a backward-compat `break_lines` fallback);
   right-click a gutter line in either shell to set a condition/logpoint. Live-verified against a
   real `debugpy` session — a conditional breakpoint on a loop correctly stopped only when `i == 3`.
9. **Web app: LSP/DAP/Leo/git in the pure client-side mode** (P2) — currently these only work in
   backend-connected mode; wiring them to the WebSocket transport closes the biggest `web/` gap.
10. **Auto-update download + install + restart** (P2) — the checker exists (§75.49); completing
    the apply path (once code signing lands) makes updates real, not just detected.

---

## Editor & language intelligence

| Feature | Priority | Notes / grounding |
|---|---|---|
| Code folding | P1 (blocked) | Architecturally blocked by the `<textarea>` editor — needs a from-scratch editing surface to hide lines. |
| Multi-cursor / column selection | P1 (blocked) | Same `<textarea>` block — native single caret only. |
| ~~Snippets / template expansion~~ | ✅ done | Shipped — curated per-language snippets, prefix+Tab expansion with tab-stop navigation, in all three editing surfaces. User-defined snippets remain a follow-up. |
| LSP code actions / quick fixes | P1 | `codeAction` capability unwired (pyright returned empty in dev; other servers implement it — verify against rust-analyzer/tsserver). |
| LSP inlay hints | P2 | Not wired. |
| LSP semantic tokens (semantic highlighting) | P2 | Electron shells use lexical `highlight.js`; semantic coloring needs `textDocument/semanticTokens`. |
| tree-sitter highlighting in Electron shells | P2 | Reuse the wgpu shell's tree-sitter engine via `web-tree-sitter` (spike already proven, §75.86). |
| Incremental/windowed re-highlight | P2 | Current highlight re-tokenizes the whole document per keystroke; unmeasured cost at very large files. |
| ~~LSP call hierarchy (incoming + outgoing)~~ / type hierarchy | ✅ call hierarchy done; type hierarchy P2 | Incoming (Shift+Alt+H) and outgoing (Shift+Alt+O) calls both shipped in both shells (`prepareCallHierarchy` + `incomingCalls`/`outgoingCalls`, live-verified against pyright). Type hierarchy remains a P2 follow-up. |
| Bracket-pair colorization | P3 | Matching-bracket highlight exists; full pair colorization does not. |
| Minimap | P3 | Not present. |
| Formatter coverage: Kotlin (ktlint), C# (`dotnet format`), Java | P2 | No stdin/stdout filter-mode wired for ktlint/`dotnet format`; Java has no formatter configured at all (§186). |

## Debugging (DAP)

| Feature | Priority | Notes / grounding |
|---|---|---|
| DAP for C#/Kotlin/Java/Go/TS in the shells | P2 | Registry-configured; needs a program-path collection UI (only Rust/Python launch today). |
| ~~Conditional breakpoints + logpoints~~ | ✅ done | Shipped — right-click a gutter line to set a DAP `condition`/`logMessage`; live-verified against real debugpy. |
| ~~Watch expressions / REPL eval~~ | ✅ done | Shipped — a WATCH panel in both shells' DebugPanel; add an expression, it evaluates in the stopped frame via a real DAP `evaluate` (`spartan_dap::DapSession::evaluate`), re-evaluated on every stop. Live-verified against real debugpy (`total * 2` → 6, `i + 100` → 103, a bad name → a real error). |
| Data breakpoints | P3 | Not present. |
| Rope-anchored breakpoints | P3 | Line-number only; edits above a breakpoint shift it (§75.8). |

## Git & source control

| Feature | Priority | Notes / grounding |
|---|---|---|
| ~~Remote push / pull / fetch~~ | ✅ done | Shipped — Fetch/Pull/Push in both Git panels (`spartan_git` remote ops, fast-forward-only pull). Clone + interactive auth-token UI remain follow-ups (remote-branch listing is now also done, see below). |
| ~~Inline blame~~ | ✅ done | Shipped — Alt+B per-line blame gutter in both shells (`spartan_git::blame_file`). |
| GitHub layer (PRs, issues, review) | P2 | §56.3–56.4, unstarted in both shells. |
| Per-hunk / partial staging | P2 | File-level staging only. |
| ~~Discard changes~~ | ✅ done | Shipped — a ⤺ "Discard changes" action (with a confirm) on each unstaged row in both Git panels (`spartan_git::discard_changes` = `git checkout -- <path>`, restores to the index version, keeps staged changes). Live-verified + cross-checked against the git CLI. |
| ~~Stash UI~~ | ✅ done | Shipped — Stash (with optional message) / Pop / Apply / Drop in both Git panels (`spartan_git` stash ops). `apply` (keep-and-apply, distinct from pop which drops) + stash-message entry now landed too. |
| Merge-conflict resolution UI | P2 | None. |
| ~~Word-level diff~~ / side-by-side | ✅ word-level done; side-by-side P3 | Word-level (intra-line) highlighting shipped in both Git panels' `DiffView` (client-side LCS token diff pairs adjacent `-`/`+` runs and emphasizes only the changed words). Side-by-side layout remains a P3 follow-up. |
| ~~Remote-branch listing~~ | ✅ done | Shipped — the branch switcher now lists `refs/remotes/*` (as of the last fetch) under "Remote branches"; clicking one creates a local tracking branch and safe-checks it out (`spartan_git::checkout_remote_branch`). Live-verified with a bare remote, cross-checked against the git CLI. |
| ~~Commit amend~~ | ✅ done | Shipped — an "Amend" button (with a confirm) beside Commit in both Git panels rewrites the last commit's message and folds in staged changes without adding a commit (`spartan_git::commit_amend` via `git2`'s `Commit::amend`). Live-verified + cross-checked against the git CLI (oid changed, commit count stayed 1, staged change folded in). |
| Rebase / cherry-pick UI | P3 | None. |

## Web app (`web/`)

| Feature | Priority | Notes / grounding |
|---|---|---|
| LSP/DAP/Leo/git in pure client-side mode | P2 | Only backend-connected mode has them; wire to the WebSocket transport. |
| ~~Multi-file tabs~~ | ✅ done | Shipped — `web/App.tsx` now tracks `openTabs`/`activeIndex` (both file kinds), with a real tab bar (switch + close), live-verified against a real devserver. §75.89's single-file gap closed. |
| ~~Redo in the WASM buffer~~ | ✅ done | Shipped — `WasmDocument` now builds a real `redo_stack` layer above `Document` (same pattern as `spartan-backend`/wgpu shell); Ctrl+Shift+Z/Ctrl+Y wired into `web/Editor.tsx`, verified through the real compiled WASM module in Node. |
| Firefox/Safari support | P3 | File System Access API is Chromium-only; needs a fallback storage backend (OPFS/`lightning-fs`). |

## Leo (agent)

| Feature | Priority | Notes / grounding |
|---|---|---|
| Automated verification commands | P2 | `Verifying` is a momentary waypoint; no real test/lint command runs (§75.66). |
| Multi-turn conversation history | P2 | Chat panel is task-scoped, no history. |
| Cooperative cancellation of in-flight model calls | P2 | Cancel discards late results but doesn't kill the background thread (§75.73). |
| Sub-agent delegation | P3 | §4.4, unstarted. |
| Team / global memory tiers + compaction | P3 | Project-tier memory only, unsummarized (§75.67). |
| Live `ClaudeProvider` / `LiteLLMProvider` verification | P2 | Both structurally complete; never run against a real key/proxy in this project's history. |

## Android (task #11)

| Feature | Priority | Notes / grounding |
|---|---|---|
| Emulator / system-image management | P2 | Blocked on `/dev/kvm` in this dev env; real on a KVM-capable machine. |
| JDWP debugging | P2 | Not present (build/install/logcat are). |
| Kotlin + Jetpack Compose LSP | P2 | Only plain-Kotlin LSP today. |
| Compose preview | P3 | None. |
| Device-picker UI (multi-device) | P3 | Auto-picks first ready device. |
| logcat filtering / search / level coloring | P3 | Raw verbatim stream only. |
| Signing / release (AAB) tooling | P3 | Debug-APK build only. |

## Production, packaging & distribution

| Feature | Priority | Notes / grounding |
|---|---|---|
| Code signing (Windows/macOS/Linux) | P1 | Named unresolved on every packaging pass. Gates trustworthy installers + auto-update apply. |
| Auto-update download + install + restart | P2 | Checker exists (§75.49); apply path deferred behind signing. |
| Native application menu (File/Edit/View/Help + About) | P2 | Deliberately deferred (§240) over Edit-accelerator conflict risk; needs a live Electron launch to validate safely. |
| Renderer bundle code-splitting | P3 | Desktop renderer is a single >500 KB chunk. |
| macOS / iOS builds | P3 | No Apple-platform build in project history. |

## GUI Builder

| Feature | Priority | Notes / grounding |
|---|---|---|
| Visual style editing (color/spacing/typography controls) | P2 | Raw key/value form today. |
| Drag-and-drop on the visual canvas | P2 | Click-to-select only. |
| Component-library browser | P2 | The one remaining named MVP gap (§75.90). |
| Live-reload while typing | P3 | Refreshes on file-switch/edit-apply only. |
| Responsive / breakpoint preview | P3 | Single viewport. |
| Asset management | P3 | None. |

## Model management

| Feature | Priority | Notes / grounding |
|---|---|---|
| Cancel/stop for in-flight downloads | P2 | No cancel on HF/Ollama/LM Studio/llama.cpp pulls. |
| `desktop/` Models panel parity via a devserver connection | P2 | Some Track-A model methods live only where a devserver connects (`web/`). |
| Live Hugging Face search API | P3 | Curated list only (broad, but fixed). |
| LiteLLM proxy restart-on-crash | P3 | Detect-only; no auto-restart. |

## Terminal & sessions

| Feature | Priority | Notes / grounding |
|---|---|---|
| Concurrent multi-session monitoring | P2 | Sessions mounts one active PTY at a time. |
| UTF-8 chunk-boundary reassembly | P3 | A multi-byte char split across OS reads can drop a replacement char (§75.64). |
| PTY resize verified against a real reader | P3 | Resize IPC works; unverified against a `$COLUMNS`-reading process. |

## Accessibility

| Feature | Priority | Notes / grounding |
|---|---|---|
| Screen-reader content reading (AT-SPI Text interface) | P2 | AccessKit tree is built; the Text interface gap means content isn't read aloud (§75.34). |
| High-contrast / reduce-motion in the wgpu shell | P3 | Present in Electron shells' theme system; not the wgpu shell. |
| File-tree / Source-Control a11y nodes | P3 | Tab list + document node exist; panels don't. |

## Spartan Cloud (Track B — separate, optional)

| Feature | Priority | Notes / grounding |
|---|---|---|
| gVisor / microVM-strength isolation verified on KVM hardware | P1 (for that track) | `runc` baseline only in this env; `/api/allocate` refuses until verified (`cloud/README.md`). |
| Real Stripe billing | P2 | `EntitlementProvider` seam ready for the swap. |
| Egress allowlist proxy | P2 | Named open policy decision. |
| Multi-node worker fleet + routing | P3 | MVP is single-node. |
| Image / registry caching | P3 | Cold-start speed. |
| Org / team features, SSO/RBAC | P3 | None. |

## Mobile (`mobile/`)

| Feature | Priority | Notes / grounding |
|---|---|---|
| Backend connectivity | P2 | No backend at all yet; a real editing/agent surface needs one. |
| True backdrop blur | P3 | Needs `expo-blur` native module + a custom dev build. |
| Font customization | P3 | Scoped out (no code-editing surface yet, §75.93). |

---

## How to use this list

- Pick from **P1** first — each is high value and lands on infrastructure that already exists.
- Follow the same discipline the rest of this project uses: real implementation, `desktop/` then
  `web/`, verify via typecheck/build/clippy/tests **and** a live Playwright run with genuine
  input, document the pass, commit per feature.
- When a feature closes a gap named in `CLAUDE.md`, update that note so the two stay in sync.
