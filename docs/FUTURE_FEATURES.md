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
7. ~~**tree-sitter syntax highlighting in the Electron shells** (P2)~~ — ✅ **Shipped in both
   `desktop/` and `web/`** (`src/treeSitter.ts` in each): real `web-tree-sitter` in-process
   parsing for all 8 languages with a bundled grammar (Rust/Python/JS/TS/Go/Java/Kotlin/C#), with
   `highlight.js` kept as a genuine fallback for json/css/xml/markdown/bash and for the window
   before a grammar finishes loading. Live-verified in a real browser across all 8 in both shells.
   See CLAUDE.md's own status entry.
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
| ~~Join Lines (Ctrl+J)~~ | ✅ done | Shipped — merges the touched lines (caret's line + next, or every selected line) into one, trimming leading whitespace and inserting a single space at each seam (VS Code behavior), in all three editing surfaces. Live-verified in `web/`. |
| LSP code actions / quick fixes | P1 | `codeAction` capability unwired (pyright *declares* `codeActionProvider` with `["quickfix", "source.organizeImports"]` in its real capabilities response, but every real hand-probed request against it — a deliberate diagnostic, `source.organizeImports`, and an empty-context request — returned `[]` in this dev environment; other servers implement it richly — verify against rust-analyzer/tsserver). |
| LSP inlay hints | P2 | A real, hand-rolled capability probe against `pyright-langserver`'s own `initialize` response found `inlayHintProvider: null` — not declared at all in this environment, confirmed before any code was written (task #182's own investigation). |
| LSP semantic tokens (semantic highlighting) | P2 | The same real probe found `semanticTokensProvider: null` — not declared at all; Electron shells use lexical `highlight.js` instead. |
| `workspace/symbol` (Go to Symbol in Workspace) | P2 | A real, live probe confirmed pyright *declares* `workspaceSymbolProvider: true`, but a real `workspace/symbol` request (both a specific query and an empty one, after the real ~90s indexing wait) returned `[]` every time in this dev environment — declared but not functionally exercisable here, the same class of finding as `codeAction`. |
| Incremental/windowed re-highlight | P2 | Current highlight re-tokenizes the whole document per keystroke; unmeasured cost at very large files. |
| ~~LSP call hierarchy (incoming + outgoing)~~ / type hierarchy | ✅ call hierarchy done; type hierarchy P2 | Incoming (Shift+Alt+H) and outgoing (Shift+Alt+O) calls both shipped in both shells (`prepareCallHierarchy` + `incomingCalls`/`outgoingCalls`, live-verified against pyright). Type hierarchy remains a P2 follow-up. |
| ~~Go to Type Definition~~ | ✅ done | Shipped — Ctrl+Shift+Click in both shells requests a real `textDocument/typeDefinition` (confirmed live and unlike `workspace/symbol`/semantic tokens/inlay hints above, this capability genuinely works here — a real query against `x: int = 1` returned a real location inside pyright's own bundled `typeshed-fallback/stdlib/builtins.pyi`). Reuses the exact same `Location \| Location[] \| LocationLink[] \| null` normalization and cross-file jump machinery go-to-definition already established. Live-verified end-to-end in both `desktop/` and `web/`, screenshotted landing exactly on `class int:`. |
| Bracket-pair colorization | P3 | Matching-bracket highlight exists; full pair colorization does not. |
| Minimap | P3 | Not present. |
| Formatter coverage: Kotlin (ktlint), C# (`dotnet format`), Java | P2 | No stdin/stdout filter-mode wired for ktlint/`dotnet format`; Java has no formatter configured at all (§186). |

## Debugging (DAP)

| Feature | Priority | Notes / grounding |
|---|---|---|
| DAP for C#/Kotlin/Java/Go/TS in the shells | P2 | Registry-configured; needs a program-path collection UI (only Rust/Python launch today). |
| ~~Conditional breakpoints + logpoints~~ | ✅ done | Shipped — right-click a gutter line to set a DAP `condition`/`logMessage`; live-verified against real debugpy. |
| ~~Watch expressions / REPL eval~~ | ✅ done | Shipped — a WATCH panel in both shells' DebugPanel; add an expression, it evaluates in the stopped frame via a real DAP `evaluate` (`spartan_dap::DapSession::evaluate`), re-evaluated on every stop. Live-verified against real debugpy (`total * 2` → 6, `i + 100` → 103, a bad name → a real error). |
| ~~DAP `output` event stream (logpoints + debuggee stdout/stderr)~~ | ✅ done | Shipped (task #275). Real, previously-undiscovered data-loss bug fixed at the root: `spartan_dap`'s `wait_for`/`wait_for_stop_or_exit` silently dropped every real `output` event that arrived while waiting for `stopped`/`exited` — a new `DapClient::wait_for_collecting_output` collects them into a side channel instead of discarding them, surfaced as a new `DapUpdate::Output { category, text }` → a real `dap_output` backend event. Two real, live-confirmed cross-adapter findings: `debugpy` relays its own internal diagnostic pings as `category: "telemetry"` output (filtered server-side before ever reaching the event); a logpoint's own interpolated message arrives with the **identical** `"stdout"` category as the debuggee's genuine `print()` output, with no distinguishing marker between the two; `lldb-dap` **also** relays real debuggee stdout via `output` events, not just `debugpy` — found live, not assumed, and fixed 4 regressed tests as a direct correct consequence (a `Continue`'s very next update is no longer always the terminal one). Both shells' `DebugPanel` gained a real OUTPUT section, bounded to 500 lines per session, cleared on every fresh launch. A real UI bug was caught only by live testing, not code review: the panel was first gated behind `isLive`, which made it vanish the instant the debuggee exited — exactly when a user most wants to review it; fixed to render on content presence instead. Live-verified end-to-end in both `desktop/` and `web/` against a real `debugpy` session: a real logpoint on a 3-iteration loop fired 3 times without stopping (`iter 0`/`iter 1`/`iter 2`), and the debuggee's own real `print(total)` output (`3`) arrived through the identical mechanism, both surviving on screen after the real exit. |
| ~~Set variable (edit a value while stopped)~~ | ✅ done | Shipped (task #276). Double-click a value in the Variables panel in either shell to edit it live via a real DAP `setVariable` (`spartan_dap::DapClient::set_variable` → `DapSession::set_variable`, scoped to the current top frame's first real scope, re-derived fresh from `thread_id` on every call so it stays correct across steps). On success the session pushes a fresh `DapUpdate::Stopped` (reason `"variable_edit"`) through the exact same event path a normal stop already uses, so the Variables panel *and* every open Watch both refresh with no second, parallel mechanism. Live-verified end-to-end in both shells against a real `debugpy` session, proving the edit reaches real program execution and not just the display: setting `x` from `21` to `100` made an open `x * 2` watch go `42` → `200`, and continuing to exit made the debuggee's own `print(y)` emit `200`, not `42`. A real, named v1 scope cut: only top-scope (locals) variables are editable, not a nested field of a compound value — that needs the *variable's own* `variablesReference` as the container, which `DapVariable` doesn't carry yet. |
| Data breakpoints | P3 | Not present. |
| Rope-anchored breakpoints | P3 | Line-number only; edits above a breakpoint shift it (§75.8). |

## Git & source control

| Feature | Priority | Notes / grounding |
|---|---|---|
| ~~Remote push / pull / fetch~~ | ✅ done | Shipped — Fetch/Pull/Push in both Git panels (`spartan_git` remote ops, fast-forward-only pull). Clone + interactive auth-token UI remain follow-ups (remote-branch listing is now also done, see below). |
| ~~Inline blame~~ | ✅ done | Shipped — Alt+B per-line blame gutter in both shells (`spartan_git::blame_file`). |
| GitHub layer (PRs, issues, review) | P2 | §56.3–56.4, unstarted in both shells. |
| ~~Per-hunk / partial staging~~ | ✅ done | Shipped — a real "Stage this hunk" button on every hunk of an unstaged file's expanded diff (`spartan_git::diff_hunks`/`stage_hunk`, built on real `git2::Patch` blob-vs-working-tree diffing and `Index::add_frombuffer` splicing — the same real mechanism `git add -p` itself uses, no hand-rolled diff algorithm). Live-verified end-to-end in both Git panels (staging one of two real, well-separated hunks left the file correctly listed as both partially staged and partially unstaged; staging the second hunk moved it to fully staged) — cross-checked against the real `git diff --staged`/`git diff`/`git status` CLI output at every step. Per-line (sub-hunk) selection and unstage-a-hunk remain real, named follow-ups. |
| ~~Discard changes~~ | ✅ done | Shipped — a ⤺ "Discard changes" action (with a confirm) on each unstaged row in both Git panels (`spartan_git::discard_changes` = `git checkout -- <path>`, restores to the index version, keeps staged changes). Live-verified + cross-checked against the git CLI. |
| ~~Stash UI~~ | ✅ done | Shipped — Stash (with optional message) / Pop / Apply / Drop in both Git panels (`spartan_git` stash ops). `apply` (keep-and-apply, distinct from pop which drops) + stash-message entry now landed too. |
| ~~Merge-conflict resolution UI~~ | ✅ done | Shipped — a "Merge" button on each non-current branch row (`spartan_git::merge_branch`, real `merge_analysis`/`merge`, real conflict markers written to the working tree exactly like `git merge`); while a merge is genuinely in progress a dedicated panel lists every conflicted file with real `ours`/`theirs` content, one-click "Take ours"/"Take theirs" resolution, and a manual-edit textarea; "Complete Merge" performs a real two-parent commit once every conflict is resolved, "Abort" resets to `HEAD` (both confirmed first). Live-verified end-to-end in both Git panels (real divergent branches, a real conflict, resolution, and a real two-parent merge commit — cross-checked against the git CLI). |
| ~~Word-level diff~~ / ~~side-by-side~~ | ✅ done | Word-level (intra-line) highlighting + a unified/side-by-side (split) toggle both shipped in both Git panels' `DiffView` (client-side: LCS token diff for word highlighting; split lays paired `-`/`+` lines in two columns, context lines span both). Live-verified in `web/`. (LeoChatPanel's own simpler edit-preview DiffView is intentionally not split-capable.) |
| ~~Remote-branch listing~~ | ✅ done | Shipped — the branch switcher now lists `refs/remotes/*` (as of the last fetch) under "Remote branches"; clicking one creates a local tracking branch and safe-checks it out (`spartan_git::checkout_remote_branch`). Live-verified with a bare remote, cross-checked against the git CLI. |
| ~~Commit amend~~ | ✅ done | Shipped — an "Amend" button (with a confirm) beside Commit in both Git panels rewrites the last commit's message and folds in staged changes without adding a commit (`spartan_git::commit_amend` via `git2`'s `Commit::amend`). Live-verified + cross-checked against the git CLI (oid changed, commit count stayed 1, staged change folded in). |
| ~~Commit revert~~ | ✅ done | Shipped — a ⟲ "Revert" button (with a confirm) on each commit in both Git panels' History view creates a *new* commit undoing that commit's changes without rewriting history (`spartan_git::revert_commit` via `git2`'s `revert`; a conflicting revert is reported honestly and the repo is left unchanged). Live-verified + cross-checked against the git CLI (commit count +1, a `Revert "…"` commit added, file reverted). |
| ~~Tags (create/list/delete)~~ | ✅ done | Shipped — a 🏷 button on each commit in both Git panels' History view tags that commit (prompts for a name; lightweight), and a "Tags" section lists every tag (name + short target oid + annotated badge) with a ✕ delete (with a confirm). Backend `spartan_git::list_tags`/`create_tag` (annotated when a message is given, else lightweight; force=false so a duplicate name errors)/`delete_tag`. Live-verified + cross-checked against the git CLI (create → list → delete round trip). |
| Rebase UI | P3 | None -- cherry-pick (below) is the one piece now shipped. |
| ~~Cherry-pick UI~~ | ✅ done | Shipped -- a "Commits" toggle on every non-current branch row (local and remote-tracking) in both Git panels' branch switcher opens that branch's own real commit log (`spartan_git::list_commits_for_ref`, browsable without checking it out) with a real "Cherry-pick" button per commit (`spartan_git::cherry_pick_commit`, real `git2::Repository::cherrypick` + a real single-parent commit -- distinct from Revert's own two-parent-free shape). A cherry-pick that's already fully present on `HEAD` is a real, honest "empty" error rather than a pointless duplicate commit. Live-verified end-to-end in both shells against real divergent branches -- cross-checked against the real `git log`/`git show --stat` CLI output, confirming the exact `(cherry picked from commit ...)` trailer. Full interactive rebase remains unstarted. |

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
| ~~Automated verification commands~~ | ✅ done | Shipped — `spartan_settings::Settings.leo_verify_command: Option<String>` (default `None`, byte-for-byte unchanged §75.66 momentary-waypoint behavior); `spartan-backend`'s new `run_leo_verification_and_completion` runs a real, configured command through `Agent::run_verification` (the same timeout-bounded `Sandbox`, §264) when `leo_next_step` sees `task_complete` — a real exit 0 marks the task `Done` (with the command/exit code/stdout/stderr on the event), a real non-zero exit marks it `Failed`, the exact state `leo_retry` (§75.78) recovers from, so a failing check genuinely feeds Leo's own bounded recovery loop instead of silently passing. Wired into `desktop/`'s Settings screen (a new "Verification command" row next to Approval Mode); `web/` has no Leo UI at all to attach a Leo-specific setting to (confirmed via grep — zero `leo_next_step`/`LeoChatPanel` references anywhere in `web/src/`), so this stays desktop-only, matching every other Leo-panel feature's own existing scope. 7 new Rust tests (pass/fail/unrunnable/recovery-round-trip for the extracted function, plus a dispatch-level `settings_set` set/preserve/clear test for the field's own nested-`Option` parse), all passing; live-verified end-to-end through the real compiled `desktop/dist` served by a real running `spartan-devserver`, confirming the real `~/.spartan/settings.json` on disk persists/clears the command correctly and survives an unrelated approval-mode save. |
| ~~`run_terminal` timeout~~ | ✅ done | Shipped — `Sandbox::run_terminal` now runs with a bounded wall-clock timeout (120s default, per-call overridable), `stdin=/dev/null` (no hang on stdin), concurrent output draining (no pipe-buffer deadlock), and a process-group kill on timeout; a killed command returns `exit_code -1` + a clear "[timed out…]" note. Closes the §75.66 gap. Unit-tested (sleep-30 killed at 300ms, cat gets EOF, 200KB output captured). |
| ~~Multi-turn conversation history~~ | ✅ done | Shipped — a real, bounded (50-entry, oldest-evicted) `leo_session_history` in `BackendState`, one entry (`task`/`outcome`/`summary`/`error`/`unix_timestamp`) per real `leo_start_task` call this backend process has seen. `outcome` is `Done`/`Failed`/`Cancelled`. `Failed` is deliberately *not* recorded the instant `mark_failed()` fires — §75.78's own bounded `Failed -> Recovering -> Executing` retry loop can still revive it — it's recorded retroactively, only once a genuinely new `leo_start_task` discards it. `leo_cancel` records `Cancelled` immediately (its own transition-table guarantee means it's only ever reached from a real in-flight state). New `leo_session_history` IPC method returns entries newest-first. `desktop/`'s `LeoChatPanel.tsx` gained a real collapsible "History" section (reusing `GitPanel.tsx`'s own established section/row CSS classes and `formatAge` convention), fetched fresh on every open. `web/` has no Leo UI at all to attach this to (matching every other Leo-panel feature's own already-documented scope). 7 new Rust tests (bounding, empty-by-default, newest-first ordering, the two extracted recording helpers, a real cancel-pushes-history test, and a real retroactive-Failed-recording test), all passing (214 `spartan-backend` lib tests total). Live-verified end-to-end through the real compiled `desktop/dist` served by a real running `spartan-devserver`: two real `leo_start_task` calls against this sandbox's own unreachable Ollama both failed fast and honestly (`Connection refused`), confirming history starts empty, stays empty until a *second* task starts (retroactive recording), then shows the first task's real `Failed` entry; a third task raced a `leo_cancel` call ahead of the real connection-refused failure and landed a real `Cancelled` entry, which itself retroactively recorded the second task's own `Failed` outcome — 3 real entries, newest first, screenshotted. |
| ~~Cooperative cancellation of in-flight model calls~~ | ✅ done | Shipped — closes the exact gap §75.73 named ("this cannot forcibly kill a real background OS thread already blocked on a model call"). `ModelProvider` gained a real, default-provided `stream_completion_cancellable(request, on_delta, cancel: &AtomicBool)` method; `OllamaProvider`/`ClaudeProvider`/`LiteLLMProvider` each override it with a real per-real-chunk check inside their own `for line in reader.lines()` SSE/NDJSON loop (checked once per real line already received over the wire — can't interrupt a single blocking read still waiting on the *next* line, an honestly-named limit, matching `subprocess::wait_with_cancellation`'s own class of limit from task #268); `LmStudioProvider` delegates straight through to its inner `LiteLLMProvider`; `FailoverProvider` fans the flag through its own real per-provider retry chain (checked between providers too, so a cancellation never falls over to try the next one); `LlamaCppProvider` gets the trait's own default no-op (an honestly-named, deliberately deferred scope cut — its real generation loop is in-process CPU/GPU token sampling with no network read to interleave a check into, documented in its own doc comment). A new `ProviderError::Cancelled` variant lets a caller distinguish a real deliberate stop from a genuine provider failure. `spartan-leo::plan::generate_plan_cancellable`/`execute::next_action_cancellable` thread the flag through to each provider call, with the original `generate_plan`/`next_action` becoming thin, byte-for-byte-unchanged wrappers passing a permanently-false flag — zero ripple to any existing caller/test/the reference wgpu shell's own `leo_bridge.rs`. `spartan-backend::BackendState` gained a real `leo_cancel_flag: Arc<AtomicBool>`, minted fresh (never reset) on every real new `leo_start_task` (the same "start fresh" discipline `leo_generation` itself already uses, so a stale clone from a superseded task can never affect a new one's flag) and read by `leo_next_step`'s own background thread for the *same* task/generation; `leo_cancel` now sets it true alongside its existing generation bump, so a real cancel genuinely interrupts the real in-flight network call instead of only discarding its late result. `desktop/`'s existing Cancel button (§75.73, `LeoChatPanel.tsx`) needed zero code changes — it already just calls `leo_cancel` with no params, so it transparently gained the real interruption behavior (only its own doc comment was updated for accuracy). 8 new Rust tests across `spartan-model` (a real, live, socket-backed `TcpListener` mock-server test proving cancellation genuinely stops an in-flight stream early — fewer than all 20 real chunks arrive — plus a `FailoverProvider` cross-provider-cancellation test confirming a cancelled chain never falls over to try a second provider), 4 in `spartan-leo` (provider-level cancellation surfaces correctly through both `generate_plan_cancellable`/`next_action_cancellable`, and the non-cancellable wrappers are confirmed unaffected), and 2 dispatch-level in `spartan-backend` (`leo_cancel` genuinely sets the real flag; `leo_start_task` mints a genuinely fresh, distinct flag per task) — all passing. **What this does not confirm**: no live model-driven exercise of a real mid-stream cancellation through an actual Leo task (Ollama's own real reachability varies by session and wasn't used for this specific end-to-end path — verified instead via a real local mock HTTP server exercising the identical NDJSON streaming code path `OllamaProvider` actually runs); no cancellation for `LlamaCppProvider`'s own in-process generation (the named scope cut above) or for a `run_terminal` subprocess mid-execution (already separately timeout-bounded, task #264, not addressed by this pass). |
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

## GUI Builder — removed

The GUI Builder (the Design screen, its live esbuild-bundled canvas, and the
whole `gui-builder/` npm project behind it) was **removed from Spartan IDE**
at the user's explicit request. Everything it did — JSX/TSX AST parsing,
`StyleChange`/`PropChange`/`Reparent`/`ComponentInsert` edits, the live
sandboxed-iframe preview, click-to-select, the component palette, and
drag-to-reparent — is gone from the shipped product, not deferred or
placeholdered.

The code remains recoverable from git history (see the commits immediately
preceding the removal); nothing here is a planned feature any more, so this
table is intentionally empty rather than repopulated with the removed rows.

## Model management

| Feature | Priority | Notes / grounding |
|---|---|---|
| ~~Cancel/stop for in-flight downloads~~ | ✅ done | A real `BackendState.download_cancellations` registry (`Arc<AtomicBool>` keyed by `<source>:<event_id>`) plus a new `model_download_cancel` dispatch method — subprocess-based HF/LM Studio pulls are stopped via a new `subprocess::wait_with_cancellation` (kills the real child process), llama.cpp's own direct HTTP download checks the flag once per real read chunk and cleans up its `.part` file. Cancel buttons in both shells' Models screens. |
| ~~`desktop/` Models panel parity via a devserver connection~~ | ✅ done | Stale row, corrected — task #145 moved every Track A model-management method (`model_status`, LiteLLM proxy lifecycle, HF/LM Studio/llama.cpp downloaders) into `spartan-backend` itself, so `desktop/`'s own `ModelsScreen.tsx` has full parity with `web/`'s `ModelsPanel.tsx` through a plain backend connection — no devserver needed. |
| Live Hugging Face search API | P3 | Curated list only (broad, but fixed). |
| ~~LiteLLM proxy restart-on-crash~~ | ✅ done | Task #273: a real generation-guarded background supervisor thread (`spawn_litellm_supervisor`) polls the child process, detects a genuine crash via `ProxyProcess::is_running()`, and respawns it up to `LITELLM_MAX_AUTO_RESTARTS = 3` times via a new `litellm_proxy::attempt_restart`, unit-verified against a real externally `kill -9`'d subprocess. Opt-in via a new "Restart automatically if the proxy crashes" checkbox in both shells' Models screens, wired through a new `auto_restart` param on `litellm_proxy_start`. |

## Terminal & sessions

| Feature | Priority | Notes / grounding |
|---|---|---|
| ~~Concurrent multi-session monitoring~~ | ✅ done | Task #274: `TerminalView` gained an `active` prop for a deterministic re-fit/redraw on becoming visible again; `SessionsScreen.tsx` lazy-mounts a provider's session on first visit and keeps it alive (CSS-hidden, not unmounted) across every later tab switch, with a live-session dot indicator. Live-verified via real WebSocket frame counting: `pty_spawn` fired exactly once per provider regardless of tab-switch count. `desktop/`-only (`web/` has no Sessions/Console screen). |
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
