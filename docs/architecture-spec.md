# Spartan IDE — Full Technical & Design Specification
### Ground-up, no-VS-Code, Rust/GPU-native IDE with Leo (Claude + Ollama hybrid agent)

---

## 1. Executive Summary

Spartan IDE is a from-scratch desktop IDE built in Rust with a GPU-rendered text engine, an agent-first/editor-first dual interface inspired by Antigravity 2.0's task-transparency model, and a hybrid AI backend (Anthropic Claude for cloud reasoning, Ollama for local/private inference). The product's three legs are:

1. **Core engine** — custom rope buffer + wgpu renderer + tree-sitter, no Monaco/CodeMirror dependency
2. **Leo** — a backend-agnostic agentic layer with planning, tool-calling, checkpointing, and artifact-based trust
3. **Design View** — a two-way-synced visual GUI builder, code and canvas as one source of truth

This document goes deep on implementation for each subsystem, plus the full interaction/visual design system.

---

## 2. Core Engine Architecture

### 2.1 Text Buffer — Rope Implementation

Requirements: O(log n) insert/delete, cheap snapshotting for undo trees, thread-safe read access for background LSP/tree-sitter work while the UI thread edits.

**Design:**
- B-tree rope (not linked-list rope) — better cache locality, fewer pointer chases. Node fanout ~64 leaf bytes target, matching `ropey`'s proven design as a base, extended with:
- **Persistent (immutable) rope nodes** — each edit creates new nodes only along the modified path, old tree remains valid → undo/redo is just "point at an old root," redo tree instead of linear undo stack (supports branching undo, a genuine feature: "show me the version before I tried that refactor")
- Each buffer carries a `Vec<RopeSnapshot>` ring for the last N edits (configurable, default 500) plus periodic full checkpoints written to disk (`.spartan/history/`) for crash recovery
- Line-index cache maintained incrementally (not recomputed per keystroke) — a separate skip-list of line-start byte offsets updated on edit, invalidated only downstream of the edit point

**Concurrency model:**
- UI thread owns the "hot" rope for the active edit
- On every edit, a lock-free snapshot pointer is published (atomic swap) that background threads (tree-sitter incremental parser, LSP didChange debouncer, semantic index updater) read from — they never block typing, and never see a torn/partial edit

### 2.2 Rendering Pipeline (wgpu)

- **Glyph atlas**: SDF (signed distance field) glyph rendering — crisp at any zoom level, cheap to scale/animate (used for the plan-step motion language in Section 8)
- **Damage-region rendering**: only re-rasterize the visible viewport + a scroll buffer of ~2 screens above/below; text shaping (via `rustybuzz` or `swash`) cached per-line, invalidated only on edit
- **Frame budget target**: 16.6ms (60fps) baseline, 8.3ms (120fps) on ProMotion/high-refresh displays; keystroke-to-glyph latency target **<5ms p99** measured input-to-photon
- **Layered compositing**: text layer, selection/cursor overlay layer, diagnostics squiggle layer, and inline-completion ghost-text layer are separate GPU passes composited together — lets ghost text animate/fade without re-rasterizing real text

### 2.3 Syntax & Language Intelligence

- **tree-sitter** for incremental parsing — reparse only the edited subtree, not the whole file, even on multi-thousand-line files
- **LSP client, built in-house**:
  - Full JSON-RPC transport over stdio/socket, own request queue with debounced `didChange` (150ms idle default, configurable)
  - Custom UI for completions (not reused VS Code widget code): virtualized list, fuzzy-match highlighting rendered as part of the glyph pass for zero extra layout cost
  - Multi-server support per file type (e.g., both `rust-analyzer` and a custom Leo-aware "semantic layer" server can attach to the same buffer)
- **DAP client, built in-house**: breakpoint state lives in the rope's metadata layer (survives edits/line-shifts automatically since breakpoints attach to persistent rope positions, not raw line numbers)

### 2.4 Symbol Graph / Semantic Index (feeds Leo)

Separate from LSP — this is Spartan's own repo-understanding layer:
- On project open: tree-sitter walks all files, builds a symbol graph (defs, refs, imports, call graph) stored in an embedded **SQLite** (or `sled`/`redb` for pure-Rust embedded KV) database at `.spartan/index.db`
- Incremental updates on file save, debounced background re-index
- Symbol graph + a lightweight local embedding pass (small sentence-transformer style model, ONNX runtime, CPU-only, no GPU contention with the editor) power **semantic code search**, independent of which LLM backend is active
- This index is what Leo's `search_codebase` tool actually queries — not a raw grep, not a full-repo-dump-to-context approach

---

## 3. ModelProvider Abstraction — Deep

### 3.1 Trait Design

```rust
#[async_trait]
trait ModelProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn is_local(&self) -> bool;
    fn context_window(&self) -> usize;
    fn supports_native_tool_calling(&self) -> bool;
    fn supports_streaming(&self) -> bool;

    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Delta, ProviderError>>>>, ProviderError>;

    async fn health_check(&self) -> ProviderHealth;
}

struct CompletionRequest {
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
    system_prompt: String,
    max_tokens: usize,
    temperature: f32,
}

enum Delta {
    TextChunk(String),
    ToolCallStart { id: String, name: String },
    ToolCallArgsChunk { id: String, partial_json: String },
    ToolCallEnd { id: String },
    Stop { reason: StopReason },
}
```

### 3.2 ClaudeProvider

- Thin wrapper over the Anthropic Messages API, native tool-use blocks map 1:1 to Spartan's `ToolDefinition`/`Delta::ToolCall*` variants
- **Prompt caching**: system prompt + repo context block (symbol graph summary, project memory file) marked as cache-control breakpoints — cuts latency/cost dramatically on multi-turn agent sessions where the same repo context is resent every turn
- Model tier selection exposed to user: fast model for inline completions, frontier model for planning/agentic turns — configurable per-task-type in settings, not just globally

### 3.3 OllamaProvider

- Talks to `http://localhost:11434` (or configurable remote Ollama host — supports pointing at a beefy LAN machine, not just localhost)
- **Startup detection**: pings `/api/tags` on launch; if unreachable, Ollama options simply gray out in the model picker rather than erroring — no forced dependency
- **Model manager UI** wraps:
  - `GET /api/tags` → installed models list
  - `POST /api/pull` (streamed) → progress bar in-app for pulling new models
  - `DELETE /api/delete` → remove models to free disk
- **Curated recommendations panel**: rather than a raw Ollama library browse, Spartan ships a maintained JSON manifest (`spartan://curated-models.json`, updated via app update channel) tagging models by role: `coding-fast`, `coding-strong`, `general`, `embedding` — e.g. surfacing well-known strong local coding models prominently instead of dumping the entire Ollama library on the user
- **Tool-calling capability matrix**: maintained per-model-family in the same manifest. Models flagged `native_tools: true` use Ollama's function-calling API directly. Models flagged `native_tools: false` fall back to Spartan's **structured-output fallback scheme** (Section 3.4)
- **Context window auto-detection**: read from Ollama's `/api/show` (model's `num_ctx` and architecture metadata) rather than hardcoding — Spartan's context-trimming logic reads this live per active model

### 3.4 Structured-Output Fallback (for non-tool-calling local models)

When `native_tools == false`:
1. System prompt appends a strict output-format instruction: Leo must emit either plain text OR a fenced ` ```spartan-tool-call ` JSON block matching a fixed schema
2. A streaming JSON-partial parser (`serde_json` incremental) watches the token stream for the fence, buffers until a complete/valid JSON object is parseable, then emits it as a synthetic `Delta::ToolCallEnd`
3. If parsing fails after N tokens (malformed JSON from a weaker model), Spartan surfaces this to the user as **"Leo attempted an action but the local model's output couldn't be parsed"** with the raw text shown — never silently drops it or guesses
4. This fallback path is intentionally more conservative: local-model-driven agent runs default to **manual-approve-every-step** rather than autonomous mode, since tool-call fidelity is lower

### 3.5 Routing Engine

```rust
enum RoutingMode {
    CloudOnly,
    LocalOnly,
    Hybrid { inline_provider: ProviderId, agentic_provider: ProviderId },
    PrivacyScoped { rules: Vec<PathRule> },
}

struct PathRule {
    glob: String,          // e.g. "secrets/**", "*.env"
    forced_provider: ProviderId,  // must be a local provider
}
```

- `PrivacyScoped` rules are checked **before context assembly** — if any file matched by an active tool call intersects a forced-local rule, the entire turn is routed to the local provider even if the session default is cloud, and the UI shows a lock badge on that message so the user sees the override happened (never silent)
- Routing decisions are logged per-session in the artifact pane (Section 8) for auditability

---

## 4. Leo Agent Core — Deep

### 4.1 Agent State Machine

```
Idle → Planning → AwaitingApproval → Executing → Verifying → Done
                                    ↘ Failed → Recovering → Executing
```

- **Planning**: Leo produces an `ImplementationPlan` artifact (structured: goal, approach, file list, risk notes) before any file write. This is a real data object, not just chat text — stored, diffable, editable by the user inline
- **AwaitingApproval**: configurable per-user — some users want to approve every plan, others only want approval on destructive operations (file deletes, `git push`, schema migrations); this is a settings matrix, not a binary toggle
- **Executing**: each tool call is logged as a `TaskStep` with status; steps run sequentially by default, but independent steps (e.g., writing two unrelated files) can be marked parallel-safe by Leo's planner and executed concurrently
- **Verifying**: after edits, Leo automatically runs configured verification (lint, type-check, relevant test subset) *before* declaring done — failures loop back to `Recovering` rather than reporting false success
- **Recovering**: bounded retry loop (default max 3 attempts) with each attempt's diff kept distinct in the artifact history, not silently overwritten — user can see "attempt 1 failed because X, attempt 2 fixed it"

### 4.2 Checkpointing

- Every `Executing` phase begins with a git plumbing snapshot: `git stash create` equivalent (non-destructive, doesn't touch working tree) or a lightweight internal snapshot if the project isn't a git repo yet
- Checkpoints are addressable objects shown in the left rail's session — "Restore to before this step" is a real, tested rollback, not just "undo in editor"
- For non-git projects, Spartan maintains its own shadow version store (`.spartan/snapshots/`) using the same content-addressable-blob approach git uses internally (dedup via hash, cheap storage)

### 4.3 Memory System

Three tiers, all local files (user-owned, git-committable if desired):

| Tier | Location | Content |
|---|---|---|
| **Project memory** | `.spartan/memory/project.md` | Conventions, architecture notes, "don't do X" rules — Leo writes to this itself after user corrections ("remember not to use default exports") |
| **Session memory** | in-memory + session log | Current task context, recent tool results |
| **Global memory** | `~/.spartan/memory/global.md` | Cross-project user preferences (coding style, preferred libraries) |

- Project memory is **summarized, not dumped whole**, into the system prompt context — a background job keeps it under a token budget by compacting older entries, similar to how prompt caching keeps the "stable" portion cache-hit-friendly
- User can open memory files directly as regular text files and hand-edit them — no black box

### 4.4 Sub-Agent Orchestration

```rust
struct SubAgentTask {
    parent_session: SessionId,
    scope: Vec<PathBuf>,       // sandboxed to these paths only
    provider_override: Option<ProviderId>,
    tool_allowlist: Vec<ToolId>,
}
```

- Sub-agents are spawned with a **narrower tool allowlist and path scope** than the parent — a "Leo-Test" sub-agent can run tests and read source but cannot `git push` or delete files, enforced at the tool-execution layer, not just prompted
- Sub-agent results stream back to the parent as a nested artifact card; parent Leo decides whether to accept/merge or discard
- Concurrency limit configurable (default 3 parallel sub-agents) to avoid runaway API cost/local-GPU contention

### 4.5 Tool Execution Sandbox

- `run_terminal` executes inside a scoped subprocess with:
  - Working directory locked to project root (no `cd ..` escape without explicit user permission grant)
  - Environment variable allowlist (secrets/`.env` values NOT auto-injected into agent-run commands unless explicitly permitted per-session)
  - Timeout + output size caps (prevents a runaway `find /` from flooding context)
- `edit_file`/`create_file` tools never touch disk directly — they write to the rope's persistent-edit layer first, producing a diff artifact; only on user/auto-approval does the diff get committed to the actual file, through the same code path as a manual save (single source of truth for "what does saving mean")

---

## 5. WASM Plugin API — Deep

### 5.1 Why WASM

Sandboxed by construction, language-agnostic (Rust, C, Go, AssemblyScript, even Python-via-Pyodide-subset can target it), no native code execution risk from third-party plugins.

### 5.2 Host-Guest Interface

- Uses the **WASI Component Model** (not raw WASI) — typed interfaces defined in WIT (WASM Interface Types), so plugin authors get real typed function signatures, not just byte-buffer marshaling
- Capability-based security: a plugin's manifest (`plugin.toml`) declares required capabilities up front:

```toml
[plugin]
name = "spartan-eslint-bridge"
version = "0.1.0"

[capabilities]
filesystem = ["read"]          # no write without explicit grant
network = false
editor_api = ["diagnostics.publish", "commands.register"]
```

- Host enforces capabilities at the import-binding level — a plugin that didn't declare `network` literally has no network import available to call, not just a runtime check that could be bypassed
- User sees a permission prompt (like a mobile app install) the first time a plugin is enabled, listing exactly what it can touch

### 5.3 Extension Points

- `editor_api`: register commands, contribute panels, subscribe to buffer-change events, publish diagnostics
- `agent_api`: **plugins can register new tools for Leo** — this is a major extensibility axis (e.g., a "Jira plugin" exposes a `create_ticket` tool Leo can call, subject to the same approval flow as built-in tools)
- `theme_api`: contribute color themes / icon packs without needing filesystem access at all

### 5.4 Marketplace

- Signed plugin packages, reproducible-build verification badge (shows "built from public source, verified") as a trust signal, distinct from a plain checkmark
- Local-first option: plugins can be sideloaded from a folder for internal/enterprise tooling without any marketplace round-trip

---

## 6. GUI Builder (Design View) — Deep

### 6.1 Canvas Engine

- Runs in an isolated **WebView surface** (only place in the app using a WebView — everything else is native wgpu) since DOM/CSS layout is genuinely the right tool for a visual design canvas
- Renders actual React/Vue/Svelte components live (not a mock/approximation) via a lightweight dev-server bridge Spartan spins up per project, so what you see in Design View is pixel-identical to what runs in the browser

### 6.2 Two-Way Sync Mechanism

This is the hard engineering problem — here's the concrete approach:

1. **Code → Canvas**: file watcher + incremental Babel/SWC AST parse on save → component tree diffed against last-known tree → canvas re-renders only changed nodes (not full reload) via HMR
2. **Canvas → Code**: every visual edit (move, resize, style change, prop edit) is captured as a structured **CanvasEdit** event, not raw pixel deltas:

```rust
enum CanvasEdit {
    StyleChange { node_id: NodeId, property: String, value: String },
    PropChange { node_id: NodeId, prop: String, value: JsonValue },
    Reparent { node_id: NodeId, new_parent: NodeId, index: usize },
    ComponentInsert { parent: NodeId, index: usize, component: ComponentRef },
}
```

3. `CanvasEdit` events are applied to the **AST directly** (via `swc`'s mutable AST API), not string-templating — preserves formatting, comments, and existing code structure the user wrote by hand
4. Codegen writes back through the **same rope-edit pipeline** as Leo's file edits (Section 4.5) — so a visual change produces a real diff artifact, reviewable/revertible exactly like an agent edit. This unifies "Leo edited this" and "I dragged this in the canvas" under one diff/undo system

### 6.3 Design Tokens

- `theme.tokens.json` as the single source of truth (colors, spacing scale, type scale, radii)
- Token panel edits write to this file; both Tailwind config generation and raw CSS-variable generation are supported as export targets, chosen per-project
- Leo-assisted design requests ("make this feel more premium") operate on the token file + component props together, producing one combined diff artifact spanning design + code

### 6.4 Import Pipeline

- Figma API import: pulls frame layout + design tokens, generates a component scaffold, flagged clearly as "unreviewed scaffold" until a human or Leo pass cleans it up
- Screenshot/mockup → component: routes the image to Claude's vision capability, generates a first-pass component + Leo immediately opens a plan artifact proposing refinements rather than treating the first output as final

---

## 7. Data Model & Storage

```
.spartan/
├── index.db              # symbol graph, embeddings (sled/redb)
├── memory/
│   └── project.md
├── snapshots/             # content-addressed blobs (non-git fallback)
├── history/                # rope checkpoint snapshots
├── sessions/
│   └── <session_id>.jsonl  # append-only event log: plans, steps, diffs, artifacts
└── config.toml             # routing mode, privacy rules, model prefs
```

- Session logs are **append-only JSONL** — crash-safe (partial writes just truncate the last line, never corrupt history), trivially diffable/greppable, and human-readable if a user wants to audit what Leo did without opening the app
- No telemetry leaves the machine by default; opt-in only, and privacy-scoped sessions never generate any network log entries at all

---

## 8. Interface & Visual Design System — Deep

### 8.1 Layout Grid & Spacing

- 8px base spacing scale (`4, 8, 12, 16, 24, 32, 48, 64`)
- Three-column skeleton (left rail / center stage / auxiliary pane) with resizable, snap-to-preset widths (collapsed / compact / expanded) rather than freeform drag-only

### 8.2 Color System

- Base surface: near-black `#0B0B0D`, elevated surfaces step up via a 4-step tint scale (`#0B0B0D → #141416 → #1C1C1F → #262629`)
- Single accent (bronze/crimson per earlier branding note, e.g. `#C4432B`) used **only** for: active mode indicator, Leo's message accent, primary action buttons — deliberately restrained so it reads as intentional, not decorative
- Semantic colors kept separate from brand accent: diagnostics red, git-add green, git-remove red-orange, warning amber — desaturated relative to the brand accent so they don't compete visually

### 8.3 Typography

- UI chrome: a geometric grotesk (e.g., licensed custom cut or a distinct open-source pairing — avoid Inter/system defaults that read as "generic AI app")
- Code/monospace: a distinct coding font with real italics support (for comments) and clear glyph disambiguation (0/O, 1/l/I)
- Type scale: modular, 1.25 ratio, base 13px for code, 14px for UI body

### 8.4 Motion Language

- Plan-step tracker: steps animate left-to-right with a spring curve (`stiffness: 300, damping: 30` equivalent), not linear easing — reinforces the "watching Leo think" feeling without being distracting
- Mode toggle (Agent/Editor/Design): center stage content cross-fades + slightly scales (0.98 → 1.0) on switch, ~180ms — fast enough to not feel laggy, slow enough to register as intentional
- Diff card accept/reject: satisfying micro-interaction (card collapses with a checkmark morph on accept) — small but this is where "feels premium" lives

### 8.5 Artifact Card Anatomy (Auxiliary Pane)

```
┌─────────────────────────────────┐
│ ● Implementation Plan      [···] │  ← status dot, overflow menu
│ Refactor auth to use JWT         │  ← title
│ 4 files · 2 risk notes           │  ← summary metadata
│ [Approve]  [Edit]  [Reject]      │  ← inline actions, no modal needed
└─────────────────────────────────┘
```

- Every artifact type (Plan, Task List, Diff, Verification Result, Sub-agent) shares this card shell for visual consistency, differing only in the expanded-state content
- Comment affordance: hover any card → a `+` appears on the right edge → click to drop an inline comment pinned to that artifact, visible to Leo on next turn

### 8.6 Empty/Onboarding States

- First launch: no fake sample project — a real guided flow that opens Design View or Editor View on an actual starter template the user picks, with Leo introducing itself in-context ("I see this is a Next.js project, want me to explain the structure or just start building?") rather than a static tutorial modal

---

## 9. Security Model

- File write approval is enforced at the **tool-execution layer in Rust**, not just as a UI suggestion the model could theoretically bypass — even if a prompt injection tricked Leo into "deciding" to skip approval, the execution layer still gates on the stored approval-mode setting
- Plugin capability sandboxing per Section 5.2
- Local-model outputs are never trusted with elevated destructive actions (`git push --force`, `rm -rf`, migrations) without explicit approval, regardless of routing mode or autonomy setting
- Secrets scanning pass runs before any diff is shown to Leo as context (redacts likely API keys/tokens from what gets sent to a cloud provider, even accidentally)

---

## 10. Performance Targets

| Metric | Target |
|---|---|
| Keystroke-to-glyph latency | <5ms p99 |
| Cold start | <800ms to interactive editor |
| File open (10k LOC) | <100ms syntax-highlighted |
| Symbol graph incremental update | <200ms per file save |
| Agent plan generation (cloud) | <3s to first plan artifact |
| Local inline completion (7B model, consumer GPU) | <150ms to first token |

---

## 11. Build Phases (Revised, Concrete)

| Phase | Milestone | Exit Criteria |
|---|---|---|
| **0** | Rope + wgpu renderer spike | Open/edit a 50k-line file at <5ms input latency |
| **1** | tree-sitter + LSP client | Completions/diagnostics working for 2 languages |
| **2** | ModelProvider (Claude + Ollama) | Ask-mode chat works on both backends, routing toggle functional |
| **3** | Leo agentic core | Plan → approve → execute → verify loop, checkpointing, diff artifacts |
| **4** | Three-column UI + mode toggle | Agent/Editor views share state, artifact cards live |
| **5** | DAP + Git panel | Breakpoints survive edits, visual diff/merge UI |
| **6** | WASM plugin API v1 | 3 reference plugins (linter bridge, theme, custom Leo tool) |
| **7** | Design View MVP | Two-way sync for React, one round-trip: drag → code diff → accept |
| **8** | Hybrid routing + privacy scoping | Path-based forced-local rules functioning, audit log |
| **9** | Polish + motion pass | Full visual design system applied, onboarding flow |

---

## 12. Open Risks / Decisions Still Needed

- **Custom UI framework maintenance cost**: building immediate-mode Rust UI for everything except Design View is a real long-term investment — confirm team has Rust/graphics depth before committing past Phase 0
- **Local model quality variance**: fallback structured-output mode (3.4) needs real-world testing against a matrix of popular Ollama models before shipping — quality will vary a lot
- **WASM Component Model tooling maturity**: verify current toolchain support for your target plugin languages before locking the plugin API surface
- **WebView isolation for Design View**: confirm this doesn't reintroduce the "feels like two apps" problem — needs careful state-sharing design so it doesn't feel bolted on

---

## 13. Enhanced AI Coding Features (Expanded)

### 13.1 Inline Intelligence

- **Multi-suggestion ghost text**: instead of one autocomplete guess, hold a modifier to cycle 3 ranked completions inline (SDF glyph layer makes this cheap to fade between, per Section 2.2)
- **Diff-aware completions**: while mid-refactor, inline suggestions are conditioned on the active `ImplementationPlan` artifact — Leo's ambient completions stay consistent with the agent's stated plan instead of drifting
- **Type-driven completion for typed languages**: LSP type info feeds directly into ranking, not just token likelihood — reduces plausible-but-wrong suggestions
- **Comment-to-code**: type a `// ` comment describing intent, tab-complete expands to a full implementation block, shown as a normal ghost-text accept (not a separate modal flow)
- **Inline "why" annotations**: hovering an AI-authored line (marked with a subtle left-gutter tick, distinct from git blame ticks) shows Leo's one-line rationale for that specific line, pulled from the originating plan artifact

### 13.2 Review & Quality Agents

- **Leo-Review sub-agent**: runs automatically on every diff before it's shown to you — a second pass that checks the *first* pass's own work for logic errors, security smells, and style-guide violations, flagged as a distinct "Self-Review" artifact card sitting above the diff
- **Regression risk scoring**: each diff artifact gets a lightweight risk badge (low/med/high) computed from blast radius (call-graph fan-in from the symbol graph) — a one-line change to a function called in 40 places scores higher than an isolated new file
- **Flaky test detector**: tracks test pass/fail history across runs in the session log; if Leo's fix "passes" a historically flaky test, it flags this rather than declaring victory
- **Dead code / unused export finder**: background scan using the symbol graph's reference counts, surfaced as a dismissible "Codebase Health" digest rather than intrusive inline warnings

### 13.3 Deeper Agentic Modes

- **Spec-first mode**: instead of prompting Leo directly, write/paste a spec doc; Leo decomposes it into a multi-session task graph (visible as a dependency tree in the left rail) and works through it across multiple sittings, resumable
- **Bisect mode**: "this broke sometime in the last 20 commits" → Leo drives an automated `git bisect`, running your test/repro command at each step, reporting the culprit commit with an explanation
- **Migration agent**: point at a codemod-style task (e.g., "migrate from Redux to Zustand") — Leo builds a file-by-file task list, applies consistent patterns across files, and pauses for review at natural checkpoints (every N files or on first divergent case) rather than one giant diff
- **Cross-repo agent** (multi-folder projects, mirroring the "Project" concept from Antigravity): Leo can reason across a frontend + backend repo pair simultaneously, with tool calls scoped per-repo but a shared plan artifact spanning both

### 13.4 Explainability Tools

- **"Why did you do that?" button** on any completed task — regenerates a plain-language postmortem from the session log, useful for PR descriptions or onboarding teammates
- **Confidence indicator** on agent claims that touch business logic vs. pure syntax — Leo self-flags lower-confidence reasoning (e.g., inferred requirements vs. explicit ones) rather than presenting everything with equal certainty

---

## 14. Additional Developer Tooling

- **Performance profiler panel**: flame graphs for supported runtimes (Node, Python via sampling profilers, Rust via `perf`/`samply` integration), with a "Leo, why is this slow?" button that hands the flame graph + hot path directly to the agent as tool context
- **Dependency graph visualizer**: interactive force-directed graph of module/package dependencies, click-to-highlight circular dependencies, exportable as an artifact for docs
- **API client panel** (expanded from earlier): request history, environment variable sets, auto-generates typed client code in the project's language from a saved request collection, with Leo able to write test assertions against captured responses
- **Database explorer** (expanded): visual schema diagram, safe query sandbox (read replicas preferred, write-guard confirmation for destructive queries), Leo-assisted migration generation that diffs current schema against a target model
- **Log tailing & structured log viewer**: attach to local dev server or remote log stream, filterable, with "ask Leo about this error" directly from a log line (auto-includes surrounding log context + relevant source file)
- **Container/env manager**: Docker Compose service status panel, one-click restart per service, Leo can read container logs as part of debugging tool calls
- **Changelog & release notes generator**: diffs two git refs, produces a categorized changelog (feat/fix/breaking) as a reviewable artifact, editable before publishing
- **Dependency upgrade agent** (expanded): a scheduled task (via the left-rail Scheduled Tasks feature) that runs weekly, opens a draft-PR-style artifact summarizing available upgrades with risk notes per package

---

## 15. Collaboration Features

- **Live multiplayer editing**: CRDT-based sync layer built on top of the persistent rope structure — since rope edits are already structured operations, they map cleanly onto CRDT ops rather than needing a separate merge model
- **Shared Leo sessions**: teammates join a session as observers or co-drivers; plan/diff approval can be configured as single-approver or quorum (e.g., 2 of 3 teammates must approve a destructive step)
- **Presence indicators**: cursor/selection avatars in Editor View, and a "Leo is currently working on X for Alice" status visible to the whole team in Agent View, so people don't duplicate agent work
- **Team memory**: a shared, git-committed `.spartan/memory/team.md` tier between project and personal memory (Section 4.3) — conventions the whole team's Leo instances respect
- **Async PR pre-review**: Leo posts its self-review artifact as an actual PR comment thread on GitHub/GitLab (via the git panel's integration), so human reviewers see AI-flagged risk areas inline in the normal review tool, not just inside Spartan
- **Voice annotations**: record a quick voice note pointing at a code region — transcribed and attached as a comment artifact Leo can act on

---

## 16. Interface Enhancements

### 16.1 Command & Navigation

- **Unified command palette (⌘K)**: single entry point for file nav, commands, AND natural-language Leo requests — typing a question routes straight into Agent View with that prompt pre-filled, typing a filename does fuzzy file-jump, no separate UI to learn
- **Radial quick-actions** (expanded from earlier): trackpad/pen gesture-accessible ring for common actions (accept diff, reject diff, open in editor, ask Leo to explain) when a code region is selected
- **Breadcrumb + minimap fusion**: minimap gutter shows not just code density but diff/diagnostic/AI-authored-line markers as colored ticks, so you can see "where the AI touched things" at a glance across the whole file

### 16.2 Workspace Customization

- **Named layouts** (expanded): save/restore full panel arrangements per task type ("Deep Debug" = terminal+debugger maximized, Auxiliary Pane minimized; "Design Session" = Design View + artifact pane only)
- **Focus mode**: single keystroke collapses everything but the active file + a minimal Leo input strip, for distraction-free writing
- **Multi-window support**: detach the Auxiliary Pane or Design View into its own OS window (useful for multi-monitor setups), state still fully synced through the same session store

### 16.3 Accessibility & Internationalization

- **Full screen-reader support**: since the UI is custom-rendered (not native DOM), this requires an explicit accessibility tree built alongside the wgpu render tree — non-negotiable engineering line item, not an afterthought (implement via platform accessibility APIs bridged from the Rust layer)
- **High-contrast and colorblind-safe theme variants** shipped by default, not just community themes
- **Adjustable motion**: full "reduce motion" setting disables the spring animations in Section 8.4 without breaking layout
- **Localized UI strings** with an ICU-based pluralization/formatting layer from day one, even if only English ships first — retrofitting i18n later is expensive, scaffolding it early is cheap

---

## 17. Extended GUI Builder Features

- **Component variant explorer**: for design-system-driven projects, browse all variants/states (hover, disabled, error) of a component in a grid, edit any variant's tokens in place
- **Interaction/prototype mode**: wire click targets between screens/components for click-through prototyping without writing routing code yet, exportable later into real router config
- **Responsive constraint editor**: visual anchor/constraint controls that generate actual CSS (flex/grid) rather than pixel-pinned styles
- **Accessibility audit overlay in Design View**: contrast ratio checks, tap-target sizing, alt-text presence — flagged directly on the canvas as you design, not just in a separate lint pass
- **Motion/animation timeline editor**: keyframe editor for CSS/animation-library-style transitions, two-way synced to code the same way static styles are (Section 6.2)
- **Component usage heatmap**: overlay showing how often each component variant is actually used across the live app, sourced from the symbol graph, to guide design-system cleanup

---

## 18. Trust, Observability & Ops

- **Cost/usage dashboard** (expanded): per-session and per-project token/cost breakdown, cloud vs. local split, with budget alerts configurable per project
- **Full audit log export**: every routing decision, approval, and tool execution exportable as structured JSON/CSV for compliance-sensitive teams (pairs with the Enterprise tier)
- **Crash reporter with local-first triage**: crash dumps are inspected locally first with an option to redact before any optional upload — never auto-uploads raw crash data silently
- **Update channel control**: explicit stable/beta/nightly channel choice, with the ability to pin a version and disable auto-update entirely — a direct lesson from the Antigravity 2.0 forced-update backlash

---

## 19. Feature Ideas Grab-Bag (for prioritization discussion)

- Jupyter-compatible notebook mode with Leo able to explain/fix individual cells
- "Rubber duck" mode — a lightweight chat panel that intentionally *doesn't* write code, only asks Socratic questions, for when you want to think, not offload
- Time-travel debugger integration (record/replay) for supported runtimes, with Leo able to jump to a specific replay point when diagnosing
- Snippet/prompt library shared across team, versioned like code
- "Explain this codebase" onboarding mode generating a navigable architecture doc + Design View component map together
- Terminal AI co-pilot: natural-language-to-shell-command inline in the terminal panel, with a dry-run preview before executing
- License/dependency compliance scanner integrated into the dependency graph panel
- Custom keybinding sets shipped for switchers (VS Code, JetBrains, Vim, Emacs bindings selectable at onboarding)
- **Language Profile Conformance Certifier** (informed directly by §47.7's real finding): when a `LanguageProfile` (§20.1) is added or its LSP/DAP command changes, run a scripted conformance probe against the actual server/adapter binary — open a fixture project, set a breakpoint, confirm hit + variable inspection; open a file, confirm diagnostics/completion/hover — and surface a pass/fail certification badge in the Languages & Toolchains settings panel, rather than assuming a new adapter behaves like the ones already integrated. §47.7 found a real deadlock (a DAP adapter deferring its `launch` response past `configurationDone`, which a different adapter answers immediately) purely by testing a second implementation instead of assuming the first one's protocol behavior generalized — this feature turns that one-off discovery into a standing regression check every future language-profile addition gets for free.
- **Fleet Health self-check** (§52): a periodic, user-visible probe that briefly launches each registered Fleet engine (§52.2) to confirm it still runs and speaks its expected CLI contract, surfaced as a "last verified" timestamp per engine in the External Agent Fleet settings category — catching a silently-broken third-party CLI (version bump, removed flag, auth expiry) before a user discovers it mid-task rather than after.

---

## 20. Universal Language & Compilation Support

Spartan must not hardcode assumptions for any one language. The architecture handles this through a **pluggable toolchain layer** sitting alongside the LSP/DAP clients from Section 2.3, rather than special-casing languages in the core engine.

### 20.1 Language Server & Toolchain Registry

```rust
struct LanguageProfile {
    id: LanguageId,                  // "rust", "python", "kotlin", "cpp", ...
    file_globs: Vec<String>,
    lsp_command: Option<CommandSpec>,     // e.g. rust-analyzer, pyright, jdtls, clangd
    dap_command: Option<CommandSpec>,
    build_systems: Vec<BuildSystemId>,    // cargo, gradle, cmake, make, maven, poetry, npm, go build...
    formatter: Option<CommandSpec>,
    tree_sitter_grammar: GrammarRef,
}
```

- Ships with a **curated default registry** (`languages.toml`, updated via the same update-channel mechanism as the Ollama curated-models manifest) covering the top ~40 languages/toolchains out of the box: Rust, Go, Python, JS/TS, Java, Kotlin, C/C++, C#, Swift, Ruby, PHP, Dart, Zig, Elixir, Haskell, Scala, Lua, Shell, SQL dialects, and more
- Auto-detection on project open: scans for marker files (`Cargo.toml`, `go.mod`, `pyproject.toml`, `build.gradle(.kts)`, `pom.xml`, `Package.swift`, `mix.exs`, etc.) and activates the matching `LanguageProfile` automatically, installing/prompting to install the missing LSP server or toolchain if not found on `$PATH`
- Any language without a maintained profile still gets tree-sitter syntax highlighting immediately (grammar library is broad) even before full LSP/build support is configured — degrades gracefully rather than treating unknown languages as plain text

### 20.1.1 Default Formatters — Prettier for the JS/TS/Web Ecosystem (amends §20.1)

`LanguageProfile.formatter` (§20.1) is a generic `CommandSpec` slot; this names the concrete default rather than leaving it unspecified per language. Rust's default is `cargo fmt`/`rustfmt`, already real and in use in this repo's own `spikes/` crates (every spike passes `cargo fmt --check` as of this pass). The equivalent for the JS/TS/web ecosystem — `LanguageProfile` entries for `javascript`, `typescript`, `json`, `css`, `html`, and `markdown` — defaults to **Prettier**, invoked the same way any other formatter is: on-save if the setting is enabled, or on-demand via the command palette, with no special-cased UI for this one tool.

**Applied now, not just specified**: this repository's two real `.jsx` files (`prototypes/interface-prototype.jsx`, `prototypes/signature-features.jsx`) are formatted with Prettier as of this pass — `.prettierrc.json` and `.prettierignore` added at the repo root, `npx prettier --write` run against both files, parse-checked clean before and after via esbuild. This does not create a JS build system for the prototypes (README's existing "don't invent one without discussing it first" caution stands — no `package.json`/`node_modules` added to the repo itself); it's a formatting pass using `npx`, the same way `cargo fmt` is a formatting pass and not a new build system for the Rust spikes.

### 20.1.2 Expanded Registry — Named Rather Than "and More" (amends §20.1)

§20.1's original ~40-language count left several real, actively-used toolchains folded into "and more" rather than named. A later request to add more compilers is the occasion to name them properly instead of leaving the registry's actual breadth implicit:

| Language | Compiler/toolchain | LSP |
|---|---|---|
| Dart / Flutter | `dart compile` | Dart Analysis Server |
| Nim | `nim c` | `nimlsp` |
| Crystal | `crystal build` | no mature LSP as of this pass — tree-sitter highlighting only, stated honestly rather than implying parity with fuller profiles |
| D | `dmd` / `ldc2` | `serve-d` |
| F# | `dotnet fsc` (same CLR as C#) | FsAutoComplete |
| OCaml | `dune` | `ocaml-lsp-server` |
| Clojure / ClojureScript | `clj` | `clojure-lsp` |
| Julia | `julia` (JIT) | `LanguageServer.jl` |
| R | `R CMD` | the R `languageserver` package |
| PowerShell | `pwsh` | PowerShell Editor Services (bundles its own LSP+DAP together, §32.3) |
| Perl | `perl` | `Perl::LanguageServer` |
| Fortran | `gfortran` | `fortls` |
| Groovy | `groovyc` | shares Java's `jdtls`-adjacent tooling where the project is Gradle-based |

**Two compilation *targets*, not source languages, worth naming for the same reason**: **WebAssembly via Emscripten** (compiles C/C++ to `.wasm`) and **`wasm-pack`** (compiles Rust to `.wasm`) — both produce output this project's own Playwright live-browser panel (§65) can already load and drive once compiled, so wiring these in completes a path that otherwise dead-ends at "compiles, but nothing in Spartan can run what it produced."

### 20.2 Unified Build/Run/Task Abstraction

Rather than a different UI per build tool, Spartan normalizes everything into a single **Task** model:

```rust
struct Task {
    id: TaskId,
    label: String,             // "Build (release)", "Run tests", "Deploy to device"
    command: CommandSpec,
    problem_matcher: Option<ProblemMatcherId>,  // parses compiler output into diagnostics
    dependencies: Vec<TaskId>, // e.g. "Run" depends on "Build"
}
```

- Each `LanguageProfile`/`BuildSystemId` ships a default task set (build, run, test, clean, lint), auto-discovered from the project's manifest — e.g., reads `Cargo.toml` targets, `package.json` scripts, Gradle tasks (`./gradlew tasks`), Makefile targets
- **Problem matchers** parse raw compiler/linker output (rustc, gcc/clang, javac, MSVC, gradle) into structured diagnostics anchored to persistent rope positions (Section 2.1) — so errors jump to the right place even if the file has since been edited
- Task Runner panel shows live streamed output with ANSI color support, and **Leo can read task output directly as tool context** — "the build failed, let me look" doesn't require the user to paste the error manually
- Multi-toolchain projects (e.g., a Rust core + Kotlin Android app + TypeScript web frontend in one repo) run as independent task graphs that Leo's cross-repo agent (Section 13.3) can coordinate across

### 20.3 Compilation Execution Model

- Local toolchains invoked as sandboxed subprocesses (same sandbox model as Section 4.5's terminal tool) — no toolchain-specific special-casing at the security layer
- **Remote/containerized build support**: for languages/targets that don't compile well natively on the dev machine (cross-compilation targets, exotic embedded toolchains), Spartan can delegate a Task to a configured remote builder (SSH host or a Docker/Podman container) transparently — output streams back identically to a local run
- Incremental build awareness: where the toolchain supports it (cargo, gradle, incremental TS), Spartan surfaces cache-hit/miss info in the Task Runner panel so users understand why a "fast" vs "slow" build happened

---

## 21. Android Development Support (First-Class Target)

Android isn't treated as "just another language" — it needs its own subsystem given the SDK/emulator/device complexity involved.

### 21.1 SDK & Toolchain Management

- **Android SDK Manager panel**: install/update SDK platforms, build-tools, NDK versions, and system images directly from Spartan (wraps `sdkmanager`), with version pinning per project (`local.properties`/`gradle.properties` respected, not overridden silently)
- Auto-detects existing Android Studio/SDK installs on first Android project open rather than forcing a redundant install
- **Gradle integration**: treats Gradle as a first-class `BuildSystemId` (Section 20.2) — task discovery via `./gradlew tasks --all`, Gradle daemon kept warm across builds for speed, build output piped through a Gradle-specific problem matcher (handles both Groovy and Kotlin DSL error formats)

### 21.2 Language Support

- **Kotlin**: full LSP via `kotlin-language-server` or JetBrains' language server where licensing permits; Compose-aware — `@Composable` functions get a **live preview render** (Section 21.4) rather than being treated as plain functions
- **Java**: `jdtls`-based LSP for legacy/mixed Java-Kotlin Android projects
- **Native (NDK/C++)**: `clangd` integration for JNI boundary code, with a dedicated view showing the Kotlin/Java ↔ native call boundary (JNI signature mismatches caught as diagnostics before runtime crashes)

### 21.3 Emulator & Device Management

- **Device panel**: lists running emulators (AVDs) and connected physical devices (via `adb`) with one-click install/run/uninstall, no dropping to a terminal required
- **AVD manager** embedded — create/edit virtual devices (API level, form factor, hardware profile) inside Spartan
- **Live logcat viewer**: filterable, tag/priority-aware, integrated with the same "ask Leo about this error" flow as Section 14's log tailing — a crash in logcat can be handed to Leo with the relevant stack trace and source context auto-attached
- **On-device debugging**: DAP-based breakpoint debugging attaches to the running app process on emulator or physical device through the standard Editor View debugger UI (Section 2.3) — same interface as debugging any other language, not a separate Android-specific debugger screen

### 21.4 Compose/Layout Live Preview

- **`@Composable` live preview**: renders directly in a side panel as you edit, using an embedded preview renderer (via Compose's own preview/tooling APIs) — hot-reloads on save
- **Design View integration**: Jetpack Compose components can be dragged/edited in the same Design View canvas as web components (Section 6), with the two-way AST sync mechanism (Section 6.2) extended with a Kotlin/Compose-specific codegen backend alongside the existing JS/TS one
- **XML layout support** for legacy View-system projects: visual layout editor with constraint handles, generating standard Android XML rather than forcing a Compose migration

### 21.5 Signing, Build Variants & Release

- **Build variant switcher**: debug/release and product flavor selection surfaced as a simple dropdown, mapped to the correct Gradle task under the hood
- **Signing config manager**: keystore management with secrets kept out of plaintext project files by default (stored via OS keychain integration, referenced not embedded) — aligns with the secrets-scanning pass in Section 9 so signing keys never accidentally leak into an AI context window
- **App bundle/APK output panel**: build artifacts listed with size breakdown (useful for spotting bloat), one-click install to a connected device, and a **Leo-assisted release checklist** artifact (version bump, changelog, ProGuard/R8 rule sanity check) before generating a release build

### 21.6 Leo's Android-Specific Tooling

New tools added to Leo's belt (Section 4.5's model) specifically unlocked in Android projects:

| Tool | Purpose |
|---|---|
| `gradle_run_task` | invoke any discovered Gradle task |
| `adb_command` | scoped ADB operations (install, logcat, shell — sandboxed like other terminal access) |
| `read_logcat` | structured logcat query, not raw tail |
| `compose_preview_render` | request a rendered preview image of a Composable for visual verification, fed back to Leo as an image |
| `analyze_manifest` | parse `AndroidManifest.xml` for permission/component changes, flagged in diff artifacts since manifest changes are often security-relevant |

- Because `compose_preview_render` returns an actual rendered image, Leo can **visually verify UI changes** it makes (via Claude's vision capability) before declaring a task done — closing the loop between "wrote the code" and "confirmed it looks right," rather than trusting compile-success alone as the definition of done

---

## 22. Studio Vision — From IDE to Engineering Studio

The three-mode system (Agent/Editor/Design) becomes a **Workspace rail**: a horizontally scrollable set of pluggable Views sharing the same left-rail sessions, right-rail artifacts, and Leo backend. Code and Design remain the flagship views; everything below is a peer view on equal footing, not a bolted-on extension.

```
Workspace:  [Code]  [Design]  [Test]  [Ops]  [Data]  [Manage]   + add view
```

- Each View is internally a plugin conforming to the same WASM extension surface from Section 5 — even Spartan's own "first-party" views are built on the public plugin API, so there's no capability gap between what Anthropic-style internal teams can ship and what third parties can build
- The unifying thread across all Views is the **project graph** (Section 30) — one linked data model underneath code, infra, tests, tickets, and deployments, so Leo and the UI both reason about "the project" as a whole, not six disconnected tools sharing a window

---

## 23. SDLC / DevOps Integration (Ops View)

- **CI/CD visual pipeline editor**: reads/writes native pipeline configs (GitHub Actions YAML, GitLab CI, CircleCI) through a visual DAG editor — drag to reorder/add steps, changes write back as clean diffs to the actual config file, never a proprietary format lock-in
- **Pipeline status inline**: build/deploy status badges surface directly in the left rail per session and in the git panel per branch — no tab-switching to a CI provider's website for the common case
- **Infra-as-code as a first-class language profile** (Section 20.1): Terraform/Pulumi/CloudFormation get LSP-equivalent support (via `terraform-ls` etc.), and **`plan`/`apply` runs produce diff artifacts** in the same Auxiliary Pane pattern as code diffs — infra changes get the identical review/approve/rollback treatment as a file edit
- **Kubernetes panel**: live cluster view (pods, services, deployments), manifest editing with schema validation, log streaming per pod feeding the same "ask Leo about this error" flow used elsewhere
- **One-click cloud deploy tasks**: AWS/GCP/Azure/Vercel/Fly.io/Cloudflare targets configured as `Task` definitions (Section 20.2) — deploy is just another task in the same runner, not a separate deploy-specific UI paradigm
- **Observability panel**: lightweight built-in metrics/log dashboard, plus embed hooks for existing Grafana/Datadog dashboards via iframe-in-WebView; **Leo can correlate a recent deploy with a metrics regression** by cross-referencing the deploy task's timestamp against the metrics panel's data as tool context — genuinely useful incident-response assist, not just a pretty chart

---

## 24. Test Studio (Unified Testing View)

- **Universal test explorer**: auto-discovers tests across every configured framework in the project (Jest, pytest, JUnit, cargo test, Go test, XCTest, Espresso, Playwright, k6) into one tree, run individually or by suite, regardless of underlying runner
- **Coverage heatmap**: gutter overlay in Editor View showing line/branch coverage from the last run, color-scaled, toggleable per test type (unit vs. integration coverage shown separately)
- **Visual regression testing**: screenshot-diff runs for UI components/pages, tied directly into Design View's canvas — a failing visual test shows the pixel diff overlaid on the actual component in-canvas, not a separate image viewer
- **Load/perf testing panel**: k6-style scripted load tests with live RPS/latency graphs during the run; Leo can analyze a completed run's results and propose likely bottlenecks by cross-referencing the profiler panel (Section 14) from the same time window
- **Flaky test dashboard**: aggregates the flaky-test detector from Section 13.2 across the whole project, ranks by flakiness frequency, and lets Leo propose fixes for the worst offenders as a batch task
- **Contract/API testing**: schema-validated request/response testing against the API client panel's saved collections (Section 14), catching breaking API changes before deploy

---

## 25. Data Science & ML Engineering View

- **Notebook mode** (expanded from Section 19): Jupyter-protocol-compatible cells, GPU-aware kernel management, with Leo able to explain/fix/optimize individual cells and understand the full notebook execution state as context
- **Experiment tracking**: lightweight built-in run/metrics logger, plus first-class integration hooks for MLflow/Weights & Biases when teams already have infra there rather than forcing a migration
- **Dataset explorer**: schema/statistics profiling for CSV/Parquet/SQL sources, missing-value and distribution visualizations, sampling preview without loading full datasets into memory
- **Model registry panel**: versioned model artifacts, one-click model-card generation (architecture, training data lineage, eval metrics) as a shareable artifact
- **Leo-assisted feature engineering**: given a dataset profile, Leo proposes candidate features/transformations as a plan artifact, generates the pipeline code, and reports before/after eval deltas — same plan→execute→verify loop as regular coding tasks (Section 4.1), applied to ML workflows

---

## 26. Project & Product Management (Manage View)

- **Built-in lightweight kanban/roadmap**: task board synced bidirectionally with git branches and PRs — moving a card can create a branch; merging a PR can auto-advance the card
- **Issue tracker integrations**: two-way sync with Jira/Linear/GitHub Issues via the plugin `agent_api` extension point (Section 5.3) — Leo can read ticket context as part of planning and, on approval, **convert an `ImplementationPlan` artifact directly into tracked issues**, closing the loop between "what Leo proposed" and "what the team is tracking"
- **Roadmap timeline view**: epics/milestones visualized on a timeline, auto-updated as linked tasks complete, giving PMs a real-time view without leaving the studio (or read-only access for non-engineers via a lightweight companion view)
- **Standup digest generator**: pulls yesterday's session logs across the team (with Team Memory tier permissions, Section 15) into a draft standup summary, editable before sharing

---

## 27. Security & Compliance Studio

- **SAST/DAST scanning panel**: static analysis integrated as a background task (Semgrep-style rule engine or language-specific scanners), dynamic scanning hooks for web targets against a running dev instance
- **SBOM generation**: software bill of materials produced per build, versioned alongside releases — pairs with the dependency graph (Section 14) and license compliance scanner (Section 19)
- **Secrets vault integration**: HashiCorp Vault / cloud KMS (AWS Secrets Manager, GCP Secret Manager) as pluggable backends — project secrets are *referenced*, never stored in plaintext project files, and the secrets-scanning pass (Section 9) treats any plaintext-looking secret as a blocking finding, not a warning
- **Compliance checklist tracking**: SOC2/GDPR/HIPAA-style control checklists as living artifacts, with evidence (audit log exports, scan results) auto-attached where Spartan already has the data — genuinely reduces manual compliance-prep toil rather than being a static template
- **Dependency vulnerability monitoring**: CVE feed cross-referenced against the dependency graph, severity-ranked, with Leo able to propose and test a patched-version bump as a normal diff artifact

---

## 28. Enterprise & Organization Features

- **SSO/SAML + RBAC**: per-project and per-team role scoping — who can approve destructive agent actions, who can access which routing modes, who can view audit logs
- **Self-hosted model gateway**: a proxy layer that fronts both the Anthropic API and an internal fleet of Ollama instances (load-balanced across GPU boxes), so `ModelProvider` implementations for large orgs point at one internal endpoint rather than every developer machine talking directly outbound
- **Org-wide policy engine**: centrally enforced routing/privacy rules (Section 3.5's `PrivacyScoped` rules, but settable at an org level and non-overridable by individual users) — e.g., "no code from `/regulated` repos ever leaves the internal network," enforced regardless of a given developer's local settings
- **Centralized audit aggregation**: every team member's session logs (Section 7) stream to a central compliance store when enterprise mode is enabled, queryable for security review without requiring per-machine access

---

## 29. Ecosystem & Marketplace Expansion

| Marketplace | Contents |
|---|---|
| **Plugin Marketplace** (Section 5.4) | WASM extensions: new tools, panels, themes |
| **Template Marketplace** | Full-stack starter kits, curated by Spartan + community, spanning all target platforms (web, mobile, embedded, ML) |
| **Workflow/Playbook Marketplace** | Shareable multi-step agent playbooks — e.g., a packaged "migrate to Compose Multiplatform" playbook others can run against their own repo |
| **Integration Marketplace** | Pre-built connectors (Slack, Notion, Stripe, Jira, PagerDuty) exposing new Leo tools via the `agent_api` extension point |
| **Theme Marketplace** | Community + official color/motion themes, all subject to the accessibility contrast checks from Section 16.3 before publish approval |

---

## 30. Unified Cross-View Data Model (Project Graph)

The mechanism that makes "engineering studio" more than a UI shell around six separate tools:

```rust
enum GraphNode {
    Symbol(SymbolRef),           // from Section 2.4's index
    File(PathBuf),
    Task(TaskId),                 // Section 20.2
    Deployment(DeploymentId),     // Section 23
    TestCase(TestId),             // Section 24
    Ticket(TicketId),             // Section 26
    Artifact(ArtifactId),         // Plans, diffs, verification results
    Vulnerability(CveId),         // Section 27
}

struct GraphEdge {
    from: GraphNode,
    to: GraphNode,
    relation: EdgeType,  // "implements", "tests", "deploys", "blocks", "fixes", "depends_on"
}
```

- Stored alongside the symbol graph in `.spartan/index.db` (Section 2.4/7), incrementally updated as artifacts, tickets, deployments, and test runs occur
- This is what lets Leo answer genuinely cross-cutting questions — *"what's the status of the payments feature"* traverses linked tickets → implementing commits → their test results → their deployment status → any open vulnerabilities on touched dependencies, and synthesizes one answer instead of the user manually checking six panels
- Also powers the Roadmap timeline (Section 26) and the "component usage heatmap" (Section 17) from the same underlying graph rather than maintaining separate ad-hoc indexes per feature

---

## 31. Updated Build Phases (Studio Expansion)

| Phase | Milestone |
|---|---|
| **10** | Ops View: CI/CD pipeline editor + one cloud deploy target working end-to-end |
| **11** | Test Studio: universal test explorer + coverage heatmap across 3 frameworks |
| **12** | Data/ML View: notebook mode + basic experiment tracking |
| **13** | Manage View: kanban + one issue-tracker two-way sync (e.g., GitHub Issues) |
| **14** | Security/Compliance Studio: SAST + SBOM + secrets vault integration |
| **15** | Project Graph v1: cross-view linking live for Code↔Test↔Ops |
| **16** | Enterprise: SSO/RBAC + self-hosted model gateway + org policy engine |
| **17** | Marketplace expansion: templates, workflows, integrations live alongside plugins |

---

## 32. Debugging Tool Integration — Top-Rated Debuggers, All Platforms

Section 2.3 established an in-house DAP client so Spartan isn't locked to VS Code's debugger UI. This section specifies which underlying debug engines it drives, organized as a **Debug Adapter Registry** parallel to the `LanguageProfile` registry from Section 20.1 — each entry wraps a best-in-class native debugger behind Spartan's own unified breakpoint/watch/step UI.

### 32.1 Debug Adapter Registry

```rust
struct DebugAdapterProfile {
    id: DebugAdapterId,
    language_ids: Vec<LanguageId>,
    engine: DebugEngine,             // which underlying debugger this wraps
    supports_reverse_debugging: bool,
    supports_remote_attach: bool,
    supports_core_dump_analysis: bool,
}
```

Spartan never reinvents the actual breakpoint/stepping machinery — it wraps the strongest existing engine per ecosystem and presents all of them through one consistent stepping/watch/call-stack UI, so switching between a Rust service and its Kotlin Android client mid-debug feels identical.

### 32.2 Native & Systems Languages

| Language(s) | Engine wrapped | Why this one |
|---|---|---|
| C, C++, Rust (via `rust-gdb`/`rust-lldb`) | **LLDB** (primary) / **GDB** (fallback, Linux-heavy toolchains) | LLDB has the strongest modern DAP support and better expression evaluation for C++/Rust; GDB kept as fallback for embedded/cross toolchains where LLDB support lags |
| Zig | LLDB | Zig's own tooling defers to LLDB for DWARF-based debugging |
| Nim, D | LLDB/GDB (DWARF-based) | Both compile through a C/C++-adjacent backend (Nim to C, D via `dmd`/`ldc2`) that emits standard DWARF debug info, so no new engine is needed — same path as C/C++/Rust |
| Embedded/microcontroller targets | **OpenOCD + GDB** (JTAG/SWD) | Industry-standard for hardware debugging; Spartan's Device panel (Section 21.3 pattern, generalized) extends to flash/attach embedded boards the same way it manages Android devices |
| Windows-native crash dumps (`.dmp`, any language emitting PDBs) | **WinDbg / CDB** (Microsoft's DbgEng) | §32.9 already names `.dmp` as an artifact Spartan ingests; this names the actual engine that opens and walks one — the real standard for Windows-native post-mortem analysis, not a DWARF-based tool |

### 32.3 Managed Runtimes

| Language(s) | Engine wrapped | Notes |
|---|---|---|
| Java, Kotlin | **JDWP** (Java Debug Wire Protocol) | Same engine Android Studio/IntelliJ use under the hood; Spartan's Android on-device debugging (Section 21.3) is a JDWP client, unified with the native LLDB path for JNI boundary debugging |
| C# / .NET | **netcoredbg** | Cross-platform, actively maintained, strong DAP-native support without requiring the full Visual Studio debugger engine |
| Python | **debugpy** | The de facto standard Python DAP adapter (same engine VS Code/PyCharm-adjacent tooling converged on); `pdb` kept available as a lightweight terminal fallback |
| Ruby | **ruby-debug-ide** / `debug` gem (Ruby 3.1+) | Modern Ruby ships an official `debug` gem with DAP support — preferred over older `byebug`-based bridges |
| PHP | **Xdebug 3** | Still the clear category leader for PHP step-debugging and profiling in one tool |
| Go | **Delve (`dlv`)** | Purpose-built for Go's runtime/goroutine model; generic GDB/LLDB can't properly unwind goroutines |
| Elixir/Erlang (BEAM) | **`:debugger`/`:dbg` via Erlang Distribution protocol** | BEAM's process model needs its native tracing tools rather than a DWARF-based debugger |
| Dart / Flutter | **Dart VM Service Protocol** (via `dart debug_adapter`) | Dart isn't JVM-based despite living alongside Kotlin/Java in mobile work (§21.2) — its own DAP-native adapter over the Dart VM, including Flutter widget-tree inspection, not a JDWP bridge |
| F# | **netcoredbg** (shared with C#) | F# compiles to the same CLR bytecode C# does, so the existing .NET debug engine (§32.3) applies directly — no separate adapter to build |
| Julia | **DebugAdapter.jl** | Julia's own DAP-native implementation, JIT-aware |
| R | **`vscDebugger`** (the R package of the same name) | The R community's own converged DAP adapter |
| PowerShell | **PowerShell Editor Services** | Ships LSP and DAP together from one process — no separate debug adapter to configure |
| Clojure / ClojureScript | **CIDER's nREPL debugger**, bridged into the same DAP UI | Clojure's debugging model is natively REPL-based; bridged rather than reinvented, consistent with §32's "wrap the strongest existing engine" principle |
| Perl | **`Perl::LanguageServer`'s DAP bridge** (wraps `perl -d`) | Modern Perl tooling converged on this over scripting raw `perl -d` sessions by hand |
| Bash / shell scripts | **`bashdb`** | Purpose-built Bash debugger — real breakpoints and stepping in shell scripts, not just `set -x` tracing |

### 32.4 Web & JavaScript/TypeScript

| Context | Engine wrapped |
|---|---|
| Node.js | **Node Inspector Protocol** (built on Chrome DevTools Protocol) |
| Browser-run frontend code | **Chrome DevTools Protocol (CDP)** directly — Spartan's `browser_preview` tool (Section 2, Leo's tool belt) and the debugger share the same CDP connection, so Leo-driven and human-driven debugging see the same live page state |
| Firefox-specific debugging | **Firefox Remote Debugging Protocol (RDP)** as an alternate adapter, for teams needing cross-browser parity |
| React Native | **Flipper** integration — layout inspector, network inspector, and Metro bundler logs surfaced as a dedicated panel alongside the standard JS debugger |

### 32.5 Swift / Apple Platforms

- **LLDB** (Swift has first-class LLDB integration, including its REPL/expression evaluation for Swift-specific types) — shared engine with the C/C++/Rust path, so a Swift app calling into a C library debugs seamlessly across the boundary
- **Instruments-equivalent panel**: Spartan doesn't reimplement Instruments, but exposes `xctrace`-driven traces (Time Profiler, Allocations, Leaks templates) inside the same Performance Profiler panel from Section 14, rather than forcing a separate app switch to Instruments.app

### 32.6 Memory, Leak & Sanitizer Tools

| Tool | Role |
|---|---|
| **AddressSanitizer (ASan) / UndefinedBehaviorSanitizer (UBSan)** | Compile-time instrumented sanitizers for C/C++/Rust — Spartan surfaces ASan crash reports as structured diagnostics anchored to source, not raw terminal dumps |
| **Valgrind (Memcheck)** | Deep memory-error detection where sanitizer instrumentation isn't available/desired; run as a Task (Section 20.2) with output parsed into the same diagnostics format |
| **heaptrack** | Heap profiling with an interactive flamegraph, feeding the Performance Profiler panel (Section 14) |
| **macOS Instruments (Leaks/Allocations)** | Covered via `xctrace` per 32.5 |

### 32.7 Distributed & Production Debugging

This is where Spartan goes beyond a typical single-process IDE debugger — genuinely necessary for a "complete engineering studio":

- **eBPF-based tooling** (`bpftrace`, or a Pixie-style always-on observability agent): live syscall/network/latency tracing on Linux targets without redeploying instrumented builds — surfaced in the Ops View (Section 23) as a "live trace" panel
- **OpenTelemetry trace debugging**: distributed trace waterfall viewer wired into the Observability panel (Section 23) — a slow request can be clicked straight from a trace span back to the exact source line that emitted it, using the project graph (Section 30) to resolve span→symbol links
- **Remote attach**: any managed-runtime adapter (JDWP, debugpy, netcoredbg, Delve) supports attaching to a running remote/containerized process, not just local processes — critical for "reproduce this only in staging" scenarios

### 32.8 Time-Travel / Record-Replay Debugging

- **`rr` (Mozilla's record-and-replay debugger)** wrapped for C/C++/Rust on Linux — deterministic replay with reverse-stepping, exposed through the same stepping UI with added "step backward" controls when the active session was recorded
- **Chronon-style JVM record/replay** integration path for Java/Kotlin where licensing/tooling permits
- This is the concrete backend for the **Bisect mode** (Section 13.3) and the **Time-travel debugger** grab-bag idea (Section 19) — Leo can drive a recorded session's reverse-step controls programmatically as part of an automated root-cause investigation, not just a human-only feature

### 32.9 Core Dump & Crash Analysis

- Core dump ingestion (`.core`, Windows minidumps `.dmp`, Android tombstones) opens directly into the debugger's post-mortem view — full call stack, register state, and local variable inspection without needing a live process
- **Leo-assisted crash triage**: given a core dump/tombstone, Leo can walk the stack, cross-reference the crashing symbol against recent diffs in the project graph (Section 30), and propose a root-cause hypothesis as a plan artifact — genuinely useful for "why did production crash at 3am" workflows

### 32.10 Unified Debugger UI Features (applies across every adapter above)

- **Conditional breakpoints & logpoints** (log a message without stopping execution) — same UI regardless of underlying engine
- **Data breakpoints** (break on variable/memory write) where the engine supports it (LLDB/GDB/JDWP yes, some scripting-language adapters no — capability surfaced honestly in the UI rather than showing a control that silently no-ops)
- **Watch expressions** evaluated in the paused frame's language, with type-aware rendering (structs/objects expandable, not just stringified)
- **Multi-target debugging**: debug a Rust backend, its Kotlin Android client, and a TypeScript web frontend in three synchronized debug sessions from one Spartan window, call stacks displayed side-by-side — directly enabled by the cross-repo agent concept (Section 13.3) applied to human-driven debugging, not just Leo
- **"Explain this stack trace" / "explain this crash"** button on any paused frame or core dump, handing full call-stack + local-variable context to Leo as tool input (ties back to Section 13.4's explainability tools)

---

## 33. Full ADB Integration (Complete Command Surface)

Section 21.6 introduced `adb_command` as a single sandboxed passthrough tool. That's enough for Leo to *use* ADB, but a "complete engineering studio" needs the full ADB command surface exposed as first-class UI, not just a terminal escape hatch. This section maps every major ADB capability to a concrete Spartan panel and a scoped Leo tool, extending the Device panel from Section 21.3 into a full multi-tab surface.

### 33.1 Device Panel — Expanded Tab Structure

```
Device Panel: [Overview] [Files] [Shell] [Logcat] [Processes] [Screen] [Performance] [Package Manager]
```

Every tab below wraps a specific ADB command family — the UI is the command surface, not a wrapper around one giant terminal.

### 33.2 Connection & Device Management

| ADB capability | Spartan surface |
|---|---|
| `adb devices` | Device list in the panel header, live-updating (poll or `track-devices` streaming mode), shows authorization status per device |
| `adb connect` / `adb pair` (wireless debugging, Android 11+) | **Wireless Pairing wizard** — QR-code or six-digit pairing code flow presented as a guided modal, no manual IP/port typing required |
| `adb tcpip <port>` | One-click "Switch to Wi-Fi debugging" toggle on a USB-connected device |
| `adb root` / `adb unroot` / `adb remount` | Explicit toggle in Overview tab, gated behind a confirmation (this is a destructive/security-relevant action per the approval model in Section 4.5) |
| `adb reboot` / `reboot bootloader` / `reboot recovery` / `reboot sideload` | Reboot dropdown with all four targets, each a distinct explicit action (never a single ambiguous "reboot" button) |
| `adb emu <cmd>` (emulator console) | Emulator-only controls: simulate GPS location, battery state, network condition (Wi-Fi/mobile/airplane), incoming call/SMS — surfaced as an "Emulator Controls" sub-panel only when the connected device is a virtual one |

### 33.3 Files Tab (`adb push` / `adb pull` / `adb shell ls`)

- Two-pane file browser: project files on one side, device filesystem on the other — **drag-and-drop between panes** performs `push`/`pull` under the hood, no command syntax exposed to the user
- Device-side browsing uses `adb shell ls`/`stat` parsed into a real tree view, not raw shell text
- App-scoped storage shortcuts (jump straight to an app's `/data/data/<package>/` or external files dir) for the currently-selected installed package

### 33.4 Shell Tab (`adb shell`)

- Full interactive PTY-backed shell session (not a one-shot command box) — same terminal rendering component used elsewhere in Spartan (Section 14), just piped through `adb shell`
- **Command palette shortcuts** for the shell commands developers reach for constantly, so they don't need to remember exact syntax:
  - `pm list packages` / `pm clear <pkg>` / `pm grant`/`revoke <pkg> <permission>` / `pm path <pkg>`
  - `am start -n <component>` / `am force-stop <pkg>` / `am broadcast -a <action>`
  - `dumpsys battery` / `dumpsys meminfo <pkg>` / `dumpsys activity` / `dumpsys window`
  - `input tap x y` / `input swipe x1 y1 x2 y2` / `input text "..."` / `input keyevent <code>`
  - `wm size` / `wm density` (with reset-to-default one click)
  - `settings get/put <namespace> <key>`
  - `getprop` / `setprop` (with a searchable/filterable property table rather than raw dump)
- Each shortcut opens a small form (fill in package name, coordinates, etc.) rather than requiring hand-typed flags — power users can still drop to raw shell input at any time

### 33.5 Logcat Tab (`adb logcat`)

- Already covered structurally in Section 21.3 — expanded here with full ADB logcat capability: buffer selection (`main`, `system`, `crash`, `radio`, `events`), priority filter (V/D/I/W/E/F), tag filter, and regex search, matching everything `adb logcat -b <buffer> -s <tag>:<priority>` supports natively
- `adb logcat -c` (clear) and log export-to-file both one click
- Crash-buffer entries auto-link to the debugger's core dump/tombstone view (Section 32.9) when a native crash is detected

### 33.6 Processes Tab (`adb shell ps` / `top` / JDWP)

- Live process list (`ps -A` parsed), CPU/memory columns refreshed on an interval
- `adb jdwp` — lists debuggable process IDs, with **one-click "Attach Debugger"** wiring straight into the JDWP debug adapter from Section 32.3, no manual port-forward setup required
- Force-stop / kill actions per process, gated the same way as other destructive actions

### 33.7 Screen Tab (`screencap` / `screenrecord` + live mirroring)

- `adb shell screencap` → one-click screenshot, saved to project assets or clipboard
- `adb shell screenrecord` → recording controls (with the standard time/bitrate/size-limit flags exposed as a simple form) — output pulled automatically via `adb pull` on stop, no manual retrieval step
- **Live screen mirroring** (scrcpy-protocol-style low-latency video stream, not just periodic screenshots) embedded directly in the panel — lets you interact with the physical/virtual device without touching it, clicks in the mirror translate to `input tap` commands sent back over ADB
- Mirror view is what feeds the `compose_preview_render` tool's real-device fallback (Section 21.6) — when a Compose preview needs to reflect actual runtime state rather than isolated preview rendering, Leo can request a live screen capture through this same pipeline

### 33.8 Performance Tab (`dumpsys` battery/meminfo/gfxinfo)

- Battery stats (`dumpsys batterystats`), memory (`dumpsys meminfo`), and frame timing (`dumpsys gfxinfo <pkg> framestats`) plotted as live charts, feeding the same Performance Profiler panel data model from Section 14 rather than a disconnected Android-only view
- `adb shell monkey` stress-test runner with configurable event count/seed, results (ANRs/crashes triggered) surfaced as findings Leo can triage the same way as any crash (Section 32.9)

### 33.9 Package Manager Tab (`pm` / `install` / `uninstall`)

- Install/uninstall APKs via drag-drop (wraps `adb install -r`/`install-multiple` for split APKs/app bundles) with install-flag toggles (`-r` replace, `-d` allow downgrade, `-g` grant all permissions) as checkboxes instead of memorized flags
- Full installed-package list with per-app permission viewer/editor (`pm grant`/`revoke`), storage usage, and version info
- `adb backup` / `adb restore` exposed for legacy full-backup workflows where still relevant (API-level gated, since modern Android has largely moved to Auto Backup/BackupAgent APIs)
- `adb bugreport` — one-click full bug report generation, saved as a project artifact and offered directly to Leo as crash-triage context

### 33.10 Leo Tool Expansion

The single `adb_command` tool from Section 21.6 is now backed by a full scoped tool set, each independently permissioned (a user can allow Leo to read logcat but require approval for install/uninstall, for example):

| Tool | Wraps |
|---|---|
| `adb_devices` | device list/status (read-only, always allowed) |
| `adb_shell_exec` | scoped shell command execution (sandboxed per Section 4.5's terminal model) |
| `adb_install` / `adb_uninstall` | package install/removal (destructive — approval-gated) |
| `adb_push` / `adb_pull` | file transfer |
| `adb_logcat_query` | structured log query (same as `read_logcat`, Section 21.6) |
| `adb_screenshot` / `adb_screenrecord` | visual capture for Leo's self-verification loop (Section 21.6) |
| `adb_dumpsys` | structured perf/state queries (battery, meminfo, activity) |
| `adb_input` | synthetic input events — lets Leo drive basic UI interaction for automated repro of a reported bug before proposing a fix |
| `adb_jdwp_attach` | request a debugger attach on a discovered debuggable process |

- Any tool marked destructive follows the same `AwaitingApproval` gate from Section 4.1 — Leo can *propose* `adb uninstall com.example.app` as part of a clean-reinstall debugging step, but it's a reviewable step in the plan artifact, never a silent side effect

---

## 34. Enhanced GUI Creation Studio (Design View, Deep Expansion)

Section 6 established the core two-way sync mechanism; Section 17 added a first pass of extended features. This section rebuilds Design View into a genuinely complete visual authoring environment spanning every platform Spartan targets — web, Compose/Android, and native desktop — not just a React-focused canvas with a few extra panels bolted on.

### 34.1 Multi-Framework Canvas Engine

The WebView canvas from Section 6.1 gets a **pluggable codegen backend** per target, all sharing one visual editing surface and one `CanvasEdit` event model (Section 6.2):

| Target | Codegen backend | Live render strategy |
|---|---|---|
| React / Vue / Svelte | AST mutation via `swc`/framework-specific parser | HMR dev-server bridge (existing, Section 6.1) |
| Jetpack Compose | Kotlin PSI-based AST mutation | Compose preview renderer (Section 21.4) or live device mirror (Section 33.7) |
| SwiftUI | SwiftSyntax-based AST mutation | Xcode preview provider bridge |
| Flutter | Dart AST mutation via `analyzer` package | Flutter's own hot-reload/DevTools bridge |
| Native desktop (Qt/GTK/WinUI, if targeted) | Framework-specific declarative-markup mutation (QML, XAML, etc.) | Native preview process, screenshotted into canvas at interactive frame rate |
| Raw HTML/CSS (no framework) | Direct DOM/CSSOM mutation, serialized back to source | Direct browser render, no dev-server needed |

- Switching target frameworks mid-project (e.g., prototyping a component in raw HTML, then generating the Compose equivalent) is supported as an explicit **"Port to..."** action — Leo handles the semantic translation as a plan artifact, not a blind 1:1 markup transliteration, since layout primitives don't map 1:1 across these systems

### 34.2 Component Authoring Tools

- **Component creation wizard**: define a new component's name, prop schema (typed — string/number/bool/enum/slot), and default variant directly from the canvas, generating a properly typed component file (TypeScript interface, Kotlin data class, Swift struct, etc. per target)
- **Isolated component playground** (Storybook-equivalent, built-in rather than a separate tool to configure): every component gets an auto-generated playground page listing all declared variants/prop combinations, live-editable prop controls generated from the type schema
- **Slot/children system**: visual drag targets for composable content regions (`children`/`slot`/Compose `content: @Composable () -> Unit`), so container components (cards, modals, layouts) are buildable and testable with arbitrary nested content directly in canvas
- **Prop-to-token binding**: any prop can be bound to a design token (Section 34.4) instead of a hardcoded value with one click, keeping component instances theme-consistent automatically

### 34.3 Layout & Responsive Design Engine

- **Auto-layout system** (Figma-style): stack/wrap/space-between behaviors settable visually, generating real flex/grid CSS or Compose `Row`/`Column`/`Arrangement` equivalents — never pixel-pinned absolute positioning unless explicitly chosen
- **Breakpoint manager**: define named breakpoints per project (not just default mobile/tablet/desktop), preview and edit each breakpoint's layout independently, with a visual diff showing what changes between adjacent breakpoints
- **Container queries support**: component-level responsive rules (not just viewport-level), visually authored the same way as breakpoints but scoped to a component's own container size
- **Adaptive layouts for Android**: window-size-class-aware canvas modes (compact/medium/expanded, per Android's own adaptive layout guidance), plus foldable-aware preview (unfolded/folded/tabletop posture) as selectable canvas states
- **Grid/flex visual editors**: direct manipulation of gap, alignment, and track sizing with live numeric readouts, generating clean, minimal CSS/Compose output rather than verbose inline styles

### 34.4 Design System Management

- **Multi-tier token model**: primitive tokens (raw values) → semantic tokens (`color.background.primary`) → component tokens (`button.background.default`) — edits at the semantic layer propagate without touching primitives, matching how mature design systems (e.g., Material, large product design systems) actually structure tokens
- **Multi-brand / multi-theme support**: manage several token sets (e.g., light/dark, or entirely different brand skins for white-label products) as named theme variants, canvas preview switchable instantly between them
- **Component library governance**: when a shared design-system component's API changes, Design View flags every usage site across the project graph (Section 30) as needing review, and Leo can propose an automated codemod migrating call sites to the new API as a batch diff artifact — design-system evolution doesn't silently break consumers
- **Deprecation warnings inline in canvas**: a deprecated component/variant shows a visible badge in both canvas and code view, with a suggested replacement one click away

### 34.5 Asset & Media Pipeline

- **Drag-in asset optimization**: images auto-compressed/resized/converted to modern formats (WebP/AVIF for web, appropriate density buckets for Android `drawable-*` folders) on import, with a manual override always available
- **SVG icon management**: icon library panel with search, automatic sprite/symbol generation for web, Vector Drawable conversion for Android, SF Symbols-style catalog integration for Apple targets
- **Inline vector editing**: basic path editing (nodes, boolean ops, stroke-to-fill) directly in canvas for simple icon tweaks — not a full illustration tool, but enough to avoid round-tripping to an external vector editor for small fixes
- **Font management**: subsetting for web performance, variable font axis controls exposed visually (weight/width/optical size sliders) where the loaded font supports it

### 34.6 Animation & Motion Design

- **Spring physics editor**: visual curve editor for spring-based transitions (stiffness/damping/mass, or duration/bounce depending on the target platform's animation API), live-previewable in canvas without a full app reload
- **Gesture-driven prototypes**: define drag/swipe/long-press interactions between canvas states for click-through (now gesture-through) prototyping, extending the interaction/prototype mode from Section 17
- **Lottie/Rive integration**: import and preview complex vector animations directly in canvas, with codegen wiring the appropriate runtime player per target platform
- **Shared-element transition authoring**: visually link an element on Screen A to its counterpart on Screen B to define a shared-element/hero transition, generating the platform-appropriate transition code (View Transitions API for web, `SharedTransitionLayout` for Compose, `matchedGeometryEffect` for SwiftUI)

### 34.7 AI-Assisted Design Generation (Leo in Design View)

- **Sketch/wireframe-to-component**: a rough hand-drawn or low-fidelity wireframe image → Leo (via Claude's vision capability) proposes a structured component tree as a plan artifact, generates real components bound to existing design tokens rather than inventing new arbitrary values
- **Text-to-layout**: "build a pricing page with three tiers, monthly/yearly toggle" → full page scaffold using existing library components where they fit, flagged clearly as a first-pass scaffold for review (same "unreviewed scaffold" convention as the Figma import path, Section 6.4)
- **Style transfer / brand alignment**: "make this match our brand" analyzes the existing token set and component library, then proposes token/style adjustments rather than freeform redesign — respects the design system instead of working around it
- **Accessibility auto-fix suggestions**: building on the audit overlay (Section 17), Leo can propose concrete fixes for flagged issues (contrast, tap target size, missing labels) as a normal diff artifact, not just a warning left for a human to resolve manually
- **Responsive variant generation**: given a designed desktop layout, Leo proposes tablet/mobile adaptations respecting the auto-layout system (34.3) rather than naive scaling, presented as additional breakpoint states for review

### 34.8 Real Data Binding & State Preview

- Canvas components can bind to **real API responses** (via the API client panel's saved collections, Section 14) or structured mock data, so the design reflects actual data shapes rather than lorem-ipsum placeholders
- **State preview switcher**: toggle any bound component between loading / empty / error / populated / edge-case (very long text, zero items, max items) states directly in canvas — catches real layout bugs before they reach a human tester
- Mock data generation assisted by Leo from a schema (OpenAPI spec, GraphQL schema, or an inferred TypeScript/Kotlin type) rather than requiring hand-written fixtures

### 34.9 Design QA & Handoff

- **Spec/measurement inspector**: click any element for precise spacing, sizing, color, and typography values — always sourced from the live token/code state, never a stale exported spec that's drifted from the real implementation
- **Code-to-design drift detection**: background check comparing canvas-authored intent against what's actually shipped in code (e.g., someone hand-edited a component's CSS outside Design View) — flagged as a reconciliation artifact rather than silently diverging
- **Pixel-diff against external design sources**: for teams still using Figma as source-of-truth during a migration period, a diffing view compares the imported Figma frame against the live rendered component, useful for verifying fidelity before fully cutting over to Design View as the primary tool

### 34.10 Collaboration in Design View

- **Real-time multiplayer canvas editing**, using the same CRDT layer from Section 15 extended to the canvas's node tree rather than just text
- **Comment threads pinned to canvas elements** (not just artifact cards, Section 8.5) — a designer can leave feedback directly on a component instance, visible to both teammates and Leo
- **Design version history with branching**: canvas states are checkpointed the same way code is (Section 4.2) — "try this alternate layout" is a real branch you can compare side-by-side and merge or discard, not a destructive overwrite

### 34.11 Cross-Platform Preview Matrix

- A single **preview grid** showing the same screen/component simultaneously across configured breakpoints (web), device profiles (Android phones/tablets/foldables), and simulators (iOS, if targeted) — one edit updates every tile live
- Exportable as a single artifact image (a real "does this work everywhere" QA artifact) attachable to a PR or design review thread

---

## 35. Prioritization & Scoping Pass — From Vision to Buildable Roadmap

Everything above this line is the complete vision. This section is the triage: what actually ships first, what proves the product before the rest is worth building, and what should be explicitly deferred or cut. The phase tables in Sections 11 and 31 were sequencing within their own scope — this section supersedes both with one unified, prioritized roadmap across the entire spec.

### 35.1 Prioritization Criteria

Every feature area was scored against four questions, in this order of importance:

1. **Does the rest of the product depend on it?** (blocking infrastructure vs. additive feature)
2. **Is it the actual differentiator, or a table-stakes feature every competitor already has?** (Leo's agentic loop and the custom engine are the bet; a database explorer is not)
3. **What's the engineering risk/cost relative to the validation it provides?** (a CRDT multiplayer canvas is expensive to build *and* expensive to get wrong before you know if anyone wants it)
4. **Does it require an existing user base to be worth building?** (enterprise SSO, marketplaces, and team features are worthless with zero users — sequence them after adoption, not before)

### 35.2 Tier Summary

| Tier | Theme | Ships when |
|---|---|---|
| **Tier 0** | Foundation spikes — must be validated before committing further engineering | Before any feature work begins |
| **Tier 1 (v1)** | Minimum one-of-a-kind, dogfoodable IDE | First public release |
| **Tier 2 (v2)** | Studio expansion — the "complete engineering studio" promise starts materializing | Once v1 has real daily-active users |
| **Tier 3 (v3)** | Ecosystem & enterprise maturity | Once there are teams/orgs, not just individuals |
| **Tier 4 (Moonshot)** | High cost, unproven demand — revisit based on actual user requests | Opportunistic, not scheduled |

### 35.3 Tier 0 — Foundation Spikes (Before Committing)

These aren't features, they're **risk gates**. Each should be a time-boxed spike with a clear go/no-go outcome before the surrounding roadmap is trusted:

| Spike | Validates | Section |
|---|---|---|
| Rope + wgpu renderer, 50k-line file, <5ms input latency | Whether the custom-engine bet (vs. forking an existing editor) is actually achievable with available team skill/time | §2.1–2.2 |
| One LSP + one DAP wired end-to-end (e.g., Rust) | The in-house protocol client approach works before building 10 more language profiles on top of it | §2.3, §32 |
| Claude + one Ollama model both completing a real multi-turn tool-calling agent loop | The `ModelProvider` abstraction and the fallback structured-output scheme (§3.4) both actually hold up, especially the local-model fallback — this is the highest-uncertainty piece of the whole spec | §3, §4 |
| Custom immediate-mode Rust UI rendering the three-column skeleton | Whether hand-building UI in Rust is sustainable, or whether the WebView-for-everything-except-editor fallback (noted as an open risk in §12) should be adopted instead | §8, §12 |

**If any of these fail, the roadmap changes before Tier 1 starts** — this is deliberate, not overhead.

### 35.4 Tier 1 (v1) — The Minimum One-of-a-Kind IDE

**Success criteria for v1**: a developer can use Spartan as their daily driver for a real Rust/TS/Python/Kotlin project, get genuinely agentic help from Leo with visible plans and reviewable diffs, debug across native and Android targets, and build one framework's UI visually — all without needing a second IDE open. That's the bar. Everything else is v2+.

| Included | Scope note |
|---|---|
| Core engine (§2) | Full — this is table stakes for the product to exist at all |
| ModelProvider: Claude + Ollama (§3) | Full trait + both providers; curated model manifest can start small (5–10 models) and grow via config updates, not a v1 blocker |
| Leo agentic core (§4) | Plan→approve→execute→verify loop, checkpointing, project-tier memory only (skip team memory, §15, for v1) |
| Language support (§20) | Launch with 5–6 fully-wrapped profiles (Rust, TS/JS, Python, Kotlin, Java, Go) rather than all ~40 — registry architecture supports the rest being added post-launch without redesign |
| Android as first-class (§21) | SDK/Gradle/emulator/device management, Kotlin+Compose LSP, on-device JDWP debugging, Compose **preview** (inline render). Full drag-and-drop Compose canvas authoring (§34.1) is v2 — preview-only is a much smaller lift and still delivers on "codes and compiles Android" |
| Debugging (§32) | LLDB/GDB, JDWP, debugpy, Delve, Node inspector — the 5–6 adapters matching the launch language set. Time-travel (`rr`), eBPF, and core-dump AI triage are v2/v3 |
| ADB integration (§33) | Devices, Files, Shell, Logcat, Processes, Screen (screenshot + basic mirror), Package Manager tabs. Wireless pairing wizard, emulator sensor controls, `monkey`, `backup`/`bugreport` are v2 |
| Interface & design system (§8, §16.1–16.2) | Three-mode skeleton (Agent/Editor/Design), artifact cards, command palette, named layouts. Radial gestures, multi-window detach are v2 |
| GUI Builder MVP (§6) | React only, full two-way AST sync, design tokens, basic component library browser. Multi-framework (§34.1), design system governance/codemods (§34.4), motion/Lottie (§34.6) are v2 |
| Accessibility baseline (§16.3) | Screen-reader tree, high-contrast theme, reduce-motion — **not deferrable**, retrofitting is expensive; build alongside the custom UI from the start |
| Security baseline (§9) | Tool-execution-layer approval gating, secrets redaction before cloud context — non-negotiable for a product that writes/executes code |
| WASM plugin API (§5) | Core capability model + 2–3 reference plugins, enough to prove the extension point works. Full marketplace (§29) is v2/v3 |
| Crash reporter, local-first (§18) | Small but important for a v1 that will crash sometimes — ship minimal version now, not deferred |
| Version control & GitHub (§56) | Local Source Control panel (stage/diff/commit/branch/stash) in full; GitHub layer scoped to read-only PR/issue visibility + PR creation + the self-review-as-PR-comment flow (§15). This row didn't exist before §56 was written despite §11/§15/§23 all assuming a git panel was already in scope — added here rather than left implicit |

### 35.5 Tier 2 (v2) — Studio Expansion

Once v1 has real usage data, layer in the features that make "complete engineering studio" true rather than aspirational:

- Full Design View expansion: multi-framework codegen (Compose/SwiftUI/Flutter canvas authoring), design system governance, motion/animation tooling, AI-assisted generation beyond basics, real data binding (§34 remainder)
- Test Studio (§24) — universal test explorer, coverage, visual regression
- Ops View (§23) — CI/CD pipeline editor, one cloud deploy target, basic observability panel
- Additional dev tooling (§14) — profiler, dependency graph, API client, DB explorer
- Remaining language profiles (§20) grown out from 5–6 to the full ~40 via the registry, prioritized by actual user requests, not the full list at once
- Remaining ADB/debugging depth (§32/§33) — distributed tracing, remote attach, wireless pairing polish
- Basic collaboration: shared Leo sessions and presence (§15), *without* full CRDT multiplayer canvas/text yet — start with lower-risk "watch and comment" collaboration before real-time co-editing
- Project Graph v1 (§30) extended from symbol-graph-only to include Task/Test/Deployment nodes as those views come online
- Cost/usage dashboard (§18)
- Plugin marketplace goes live (§29), narrower than the full 5-marketplace vision — plugins + templates first, workflows/integrations later

### 35.6 Tier 3 (v3) — Ecosystem & Enterprise

Sequenced last because it's genuinely worthless without an existing user/team base to serve:

- Full CRDT multiplayer (canvas + text), team memory tier (§15)
- Manage View / PM layer (§26) — lower priority than Test/Ops since Jira/Linear/GitHub Issues already serve this well; two-way sync matters more than a competing built-in kanban
- Data/ML View (§25) — only build if user research shows real demand; this is a genuinely different user persona than the core dev-tool audience
- Security/Compliance Studio in full (§27) — SAST/DAST, SBOM, compliance checklists
- Enterprise (§28) — SSO/RBAC, self-hosted model gateway, org policy engine, centralized audit
- Full marketplace ecosystem (§29) — workflow/playbook and integration marketplaces
- Design QA drift detection, Figma pixel-diffing (§34.9), cross-platform preview matrix (§34.11)

### 35.7 Tier 4 (Moonshot) — Defer Until Explicitly Requested

High engineering cost relative to validated demand. Don't schedule these — revisit if users specifically ask:

- `rr`/Chronon time-travel record-replay debugging (§32.8)
- eBPF/Pixie-style always-on production tracing (§32.7) — Linux-only, heavy infra lift
- Native desktop (Qt/GTK/WinUI) GUI builder codegen backend (§34.1) — niche relative to web/mobile
- Full compliance evidence automation (SOC2/GDPR report generation, §27)
- Multi-brand/white-label theming (§34.4)
- Custom scrcpy-protocol reimplementation for screen mirroring (§33.7) — **shell out to existing `scrcpy` as a v1/v2 implementation detail instead of reimplementing the protocol**; revisit only if embedding requirements demand it

### 35.8 Explicit Cut/Reconsider List

Not "never," but flagged as scope creep risks that should require a deliberate re-justification before building, not just inertia from being in the original spec:

- Built-in kanban/roadmap competing directly with Jira/Linear — sync, don't rebuild
- Five separate marketplaces (§29) — likely collapses into 2 (Plugins+Themes, Templates+Workflows) in practice
- In-canvas full illustration/vector tooling (§34.5) beyond basic icon tweaks — scope creep toward "being Figma" instead of "being an IDE with a good canvas"

### 35.9 Critical Path & Sequencing Risks

- **Everything in Tier 1 depends on the Tier 0 spikes succeeding as designed** — most importantly the custom Rust UI spike and the local-model tool-calling fallback, since those are the two areas where "we assumed this would work" is riskiest
- **Android-as-v1 is the single biggest scope risk inside Tier 1** — SDK/Gradle/emulator/ADB/JDWP is effectively a second full toolchain integration alongside the core engine. If timeline pressure hits, the fallback is shipping v1 with strong native+web language support and Android in early v2, rather than compromising core engine quality to hit an Android deadline
- **GUI Builder's WebView-in-otherwise-native-app architecture (§6.1) needs its own integration spike**, not just a rendering spike — state-sharing between the native shell and the WebView canvas is where "feels like two apps" (the exact Antigravity 2.0 failure mode) could quietly reappear if not tested early

### 35.10 One-Line Elevator Pitch Per Tier (for internal alignment)

- **v1**: "A from-scratch, GPU-fast IDE where Leo actually shows its work, runs on your GPU or Claude's, and builds/debugges real apps including Android — no VS Code underneath."
- **v2**: "Now it's a studio — test, ops, and full-fidelity multi-platform design are first-class, not bolted on."
- **v3**: "Now it's ready for teams and enterprises to standardize on."

---

## 36. Failure Mode Analysis — Learning From Every Major IDE's Mistakes

This section catalogs documented, current failure patterns from the IDEs Spartan is most directly positioned against, root-causes each one to an architectural or product decision, and — critically — adds concrete new hardening to Spartan's design where the existing spec didn't already cover it. This isn't a competitive dig; every one of these tools is good enough to have millions of users. That's exactly why their failure modes are worth taking seriously.

### 36.1 Methodology

Reviewed current, documented pain points across three categories: AI-native agentic editors (Cursor, Windsurf, Antigravity 2.0 — Spartan's closest competitive set), JVM-based heavyweight IDEs (JetBrains suite), and extensible lightweight editors (VS Code) — plus long-standing structural failure modes common to Eclipse, Xcode, and Android Studio. Each failure is root-caused, then mapped to either an existing Spartan mitigation (cross-referenced to its section) or a new mechanism added specifically because this analysis exposed a gap.

### 36.2 Failure Catalog

| IDE | Failure Mode | Root Cause | Spartan Prevention |
|---|---|---|---|
| **Cursor** | Silent code reversions — agent-applied edits get overwritten without notification when internal features (review tab, cloud sync, format-on-save) race each other over the same file state | Multiple independent write paths to the same file with no single source of truth or write lock | **Single Writer Invariant** (§36.4.1) — every edit, agent or human, routes through one rope-edit pipeline |
| **Cursor** | Malicious cloned repo can trigger arbitrary code execution through the agent (patched CVE, but the class of bug is structural) | Agent given broad execution trust on repo open, including auto-run project config, before the user has vetted the repo | **Untrusted-Repo Quarantine Mode** (§36.4.2) |
| **Cursor** | Agent mode "over-eager rewrites" — refactors more than the user asked for | No enforced scope boundary between what a plan declares and what tool calls are allowed to touch | **Plan-Scope Enforcement** (§36.4.3) |
| **Cursor** | Escalating real-world cost vs. advertised price due to opaque per-request credit consumption | Usage-based pricing surfaced only after the fact, not before a costly operation runs | Extends existing cost dashboard (§18) with pre-flight cost estimates, detailed §36.4.4 |
| **Windsurf** | Model dropdown shows one model (e.g. a premium tier) while requests are actually routed to a different, cheaper model | Display layer for "which model is active" is decoupled from the code path that actually executes the request | **Model Integrity Guarantee** (§36.4.5) |
| **Windsurf** | Documented critical CVEs: path traversal allowing arbitrary local file read/write, and MCP configuration tampering leading to remote command execution | Tool/agent layer trusting file paths and MCP server registration without a hard boundary | **Path-Jailing + MCP Registration Approval** (§36.4.6) |
| **Windsurf** | Opaque credit system, quota cut overnight with no advance communication, billing continuing post-cancellation | Pricing/quota changes treated as a backend config change rather than a user-facing commitment | Operating principle, §36.4.7 |
| **Windsurf** | Terminal-driven agent tasks stalling or getting interrupted mid-operation, breaking flow | No clear task lifecycle/timeout/retry contract for long-running tool calls | Already covered by the `Task` execution model (§20.2) — reaffirmed, no new mechanism needed |
| **Antigravity 2.0** | Forced split into two separate apps, editor/file tree hidden by default, confusing dual-AppData directories, aggressive auto-update with no opt-out at rollout | Agent surface and editor surface built as genuinely separate products bolted together late | Already the founding design decision behind Spartan's unified Workspace rail (§8, §22) and update channel control (§18) — this was the direct inspiration, reaffirmed here |
| **Antigravity 2.0** | A later minimalism pass reportedly removed the terminal, inline error/warning indicators, and direct in-place code editing in favor of a chat-only surface — user reports describe it as gutting "the traditional IDE experience," not merely restyling it | "Simplify and declutter" applied to the wrong layer — it reduced core editing/debugging surface area instead of secondary chrome (redundant borders, badges, labels) | **Decluttering Never Removes Core Surfaces** (§36.4.10) — a named, permanent boundary on what any future visual-simplification pass is allowed to touch |
| **JetBrains suite** | UI freezes for 10+ seconds during indexing/branch switches; unbounded memory growth that can hit multiple GB even on moderate projects | Monolithic JVM-based indexer blocks the UI thread; no documented memory ceiling relative to project size | Already covered by the non-blocking, incremental background indexer (§2.4) — hardened with an explicit memory budget, §36.4.8 |
| **VS Code** | A single poorly-coded extension (synchronous blocking calls, unbounded startup activation, memory leaks) degrades the entire editor's responsiveness; marketplace has no performance gate before publish | Extensions share process boundaries with soft isolation, not hard resource limits; no vetting on activation cost | Already covered by WASM plugin sandboxing (§5) — hardened with per-plugin resource budgets and a marketplace performance gate, §36.4.9 |
| **Eclipse** (long-standing, structural) | Workspace metadata corruption forces a full workspace reimport, losing local state | Internal metadata store is mutable and can desync from actual filesystem/git state with no recovery path | Already covered — `.spartan/index.db` and session logs are append-only/content-addressed (§7); index rebuild is always possible from source truth, non-destructive |
| **Xcode** (long-standing, structural) | SwiftUI Previews frequently fail to build or crash, requiring a full rebuild and breaking flow | Preview process is tightly coupled to full project build state, fragile as a monolith | Compose/preview rendering (§21.4, §34.1) runs in an isolated, auto-restarting subprocess with a "stale preview" indicator instead of a hard failure |
| **Android Studio** (long-standing, structural) | Gradle sync hangs or takes minutes, blocking the editor meanwhile | Sync treated as a blocking foreground operation rather than an async background task | Already covered — Gradle sync is an async `Task` (§20.2) that never blocks editing |

### 36.3 Cross-Cutting Patterns

Four root causes explain nearly everything in the table above, which matters more than the individual bugs:

1. **Opacity erodes trust faster than any single bug.** Model substitution, silent reversions, and unannounced quota cuts all share one root cause: the user couldn't see what was actually happening. Spartan's answer is structural, not cosmetic — the artifact/observability model (§8.5, §18) makes every state change a visible, attributable record by construction, not an after-the-fact log a user has to go looking for.
2. **Single points of failure turn small bugs into whole-app outages.** One extension, one indexer thread, one editor process. Spartan's answer is isolation by construction: WASM plugin sandboxing (§5), independently-scheduled async Tasks (§20.2), and an index architecture where corruption is always locally recoverable from source truth (§7).
3. **Agent autonomy without a hard permission boundary is a security surface, not just a UX nuance.** Two of the three AI-native competitors above have documented CVEs stemming directly from this. Spartan's tool-execution-layer approval gate (§4.5, §9) is reinforced below with explicit path-jailing and quarantine-mode hardening specifically because "we prompted the model to be careful" is not a security boundary.
4. **Forced migrations and opaque monetization changes destroy trust overnight, independent of technical quality.** Antigravity's rollout and Windsurf's pricing backlash both show a good underlying product can still lose a user base through a single badly-handled change. Spartan's answer is explicit update-channel control (§18) plus an operating commitment to advance-notice pricing changes.

### 36.4 New Hardening Added to the Spec

These are concrete additions/amendments made directly because of this analysis — not restatements of what already existed:

**36.4.1 Single Writer Invariant** (amends §4.5, §6.2)
Every mutation to a buffer — whether from Leo's `edit_file` tool, the Design View canvas's `CanvasEdit` pipeline, or a manual keystroke — acquires the same rope-edit lock and produces a single versioned write. If two sources attempt to write the same region concurrently, the second write is held as a **Conflict artifact** (new artifact type, same card shell as §8.5) requiring explicit resolution, never silently applied or silently dropped.

**36.4.2 Untrusted-Repo Quarantine Mode** (amends §9)
On first opening any repo Spartan hasn't seen before, it defaults to a quarantine execution profile: no auto-run tasks, no auto-approved terminal commands regardless of the user's global autonomy settings, and no secrets-vault access, until the user explicitly marks the repo trusted. This applies even if the user's default approval mode is "autonomous" — quarantine mode overrides the default for unfamiliar repos specifically.

**36.4.3 Plan-Scope Enforcement** (amends §4.1, §4.5)
An `ImplementationPlan` artifact declares its intended file/symbol scope up front. Any `edit_file`/`create_file` tool call touching a path outside that declared scope requires a distinct **"Scope Expansion"** approval step, shown separately from the original plan's diffs — Leo can still propose touching more than originally planned, but it's never silently bundled into the same reviewed diff.

**36.4.4 Pre-Flight Cost Estimates** (amends §18)
Before executing an agentic task above a configurable cost/token threshold, Spartan shows an estimated cost range as part of the plan artifact itself, not just in a separate historical dashboard — the estimate is visible at the point of decision, not after the fact.

**36.4.5 Model Integrity Guarantee** (amends §3.2, §8.6)
The model badge shown on every Leo message (`Leo · Claude Sonnet`, `Leo · Qwen local`) is populated directly from the `ProviderId` returned by the `ModelProvider` implementation that actually executed the call (§3.1's trait) — never from a separate display-layer config value that could drift from what's really running. This is enforced structurally: there is no code path that lets the UI label a response with a provider that didn't handle it.

**36.4.6 Path-Jailing + MCP/Plugin Registration Approval** (amends §4.5, §5.2)
All tool-layer file path resolution is canonicalized and hard-jailed to the project root at the sandbox level — no `../` traversal, symlink escape, or absolute-path override can resolve outside it, regardless of what the model requests. **One documented, explicit exception**: an active Developer Mode session (§60.2, revised in §60.2.1) removes this jail for that specific workspace, at the user's own deliberate opt-in, with a one-time confirmation on the first path resolution outside the project directory and a persistent visible indicator for the rest of the session — this is the only place in the spec where the jail is not absolute, and it is named here rather than left as a silent contradiction between this section and §60. Separately, any change that would register a new MCP server or grant a plugin new capabilities is itself treated as a security-relevant diff requiring explicit approval — never auto-loaded silently from a config file change on next launch, and this half of the invariant has no Developer Mode exception at all.

**36.4.7 Transparent Pricing Commitment** (operating principle, ties to §18/§28 tier model)
Quota or pricing changes are communicated with advance notice before taking effect, and the in-app cost dashboard reflects the actual current terms in real time — a product-policy commitment made explicit here because it's the single most common trust-destroying failure across the AI-native competitor set, independent of any code defect.

**36.4.8 Indexing Memory Budget** (amends §2.4)
The symbol graph indexer operates against a configurable memory budget relative to project size. When a project would exceed it, indexing degrades gracefully — partial index coverage with a persistent non-blocking "still indexing" indicator — rather than consuming unbounded RAM or freezing the UI thread. The UI thread is never blocked by indexing under any circumstance, by construction (§2.1's concurrency model already guarantees this; this adds the resource ceiling on top).

**36.4.9 Per-Plugin Resource Budget + Marketplace Performance Gate** (amends §5.2, §5.4)
Each WASM plugin runs against an enforced CPU/memory budget at the runtime level, not just a capability permission list — a plugin exceeding its budget is automatically throttled or suspended rather than degrading the whole app. Marketplace publishing requires passing a performance benchmark (startup activation time, idle CPU usage) before listing, turning "no vetting" from a known VS Code gap into an actual submission gate.

**36.4.10 Decluttering Never Removes Core Surfaces** (amends §8, ties to §50.3)
Any visual-simplification pass — including §50.3's own high-contrast theme adoption — is scoped to secondary chrome only: redundant borders, boxed metadata pills, duplicated labels, excess simultaneous badges. It never removes, hides-by-default, or degrades the terminal panel (§59), inline diagnostics/breakpoint gutter (§2, §32), or direct in-place code editing (§1's editor-first half of the dual interface) in favor of a chat-only surface. This is a permanent boundary, not a one-time decision: a future pass that wants to simplify further still has to keep all three, the same way §60's Developer Mode widening kept its own two hard stops rather than treating "widen the box" as license to remove everything.

### 36.5 "Never Again" Release Gate Checklist

A short set of binary checks every release should be able to answer, operationalizing this whole section so it doesn't stay theoretical:

- [ ] Can any code change happen without appearing as an attributable, visible artifact? → must be **No**
- [ ] Can the displayed active model ever diverge from the model that actually executed a request? → must be **No**
- [ ] Can a single plugin degrade the responsiveness of the rest of the app? → must be **No**
- [ ] Can indexing or build sync ever block the main UI thread? → must be **No**
- [ ] Can an agent tool call resolve a filesystem path outside the project root? → must be **No**
- [ ] Can a pricing/quota change take effect before the user has been notified? → must be **No**
- [ ] Is there always a one-click, non-destructive way to rebuild a corrupted local index/cache from source truth? → must be **Yes**

---

## 37. Comprehensive Enhancement Pass — Every Pillar

A full pass across the whole product, adding genuinely new capabilities rather than restating what's already spec'd — organized by the same pillars used throughout this document.

### 37.1 Core Engine

- **Structural/AST-aware editing**: "smart selection" expands by syntax node (expression → statement → block → function) instead of raw text ranges; multi-cursor operations respect symbol boundaries via tree-sitter, not pattern-matching, so a multi-cursor rename can't accidentally clobber a substring inside an unrelated identifier
- **Virtual File System abstraction**: local, SSH-remote, container (Dev Container spec-compatible), and browser-based cloud workspaces all present through the same file-tree API — remote development is a VFS implementation detail, not a separate mode with its own UI
- **Monorepo workspace virtualization**: lazy-loads the symbol graph (§2.4) per accessed subtree for 100k+ file repos, with sparse-checkout awareness so indexing cost scales with what you actually touch, not the whole repo
- **Correct Unicode/bidi text handling** (RTL languages, combining characters, grapheme-cluster-aware cursor movement) built into the rope and shaping layer from Phase 0, not retrofitted — this is exactly the kind of thing that's brutal to bolt on later

### 37.2 Leo / Agent Core

- **Verification-driven mode**: Leo can write the failing test first, then implement until it passes — TDD as a selectable first-class agent mode, not just a suggestion in the plan
- **Multi-option proposal mode**: for architecturally significant changes, Leo presents 2–3 distinct implementation approaches with explicit trade-offs as parallel plan artifacts *before* writing any code, so direction gets picked before effort is spent
- **Local preference learning**: a lightweight per-user ranking signal (not a full model) learns from accept/reject/edit patterns on Leo's diffs to bias future suggestions toward the user's actual style — stored locally by default, only entering the shared Team Memory tier (§15) if explicitly opted in
- **Graceful offline degradation**: when neither cloud nor local model is reachable, Spartan falls back to LSP-only assistance (completions, diagnostics) with a clear "agent features unavailable" state — never a silent hang or an unclear error
- **Consensus mode for high-risk changes**: two model configurations independently propose a fix for security-sensitive or infra-destructive changes; Leo surfaces where they agree vs. diverge as a structured comparison artifact — extra scrutiny exactly where the blast radius is highest

### 37.3 Language & Build

- **Distributed compilation caching** (sccache/ccache/Bazel-remote-cache-style) wired into the Task runner (§20.2) — a team-shared build cache that meaningfully speeds up clean builds across everyone, not just the machine that built it first
- **WASM as a first-class compile target** alongside native and Android — coherent with the plugin system already running on WASM (§5)
- **Cross-compilation matrix dashboard**: one view showing build status across every configured target (e.g., x86_64-linux, aarch64-macos, wasm32, android-arm64) after a single commit, instead of checking each target separately

### 37.4 Mobile Platforms, Beyond Android

- **iOS promoted to equally first-class**, generalizing the Section 21 pattern: `xcodebuild`/`swift build` wrapped the same way Gradle is wrapped, a Simulator management panel mirroring the Android Device panel, and a provisioning-profile/signing manager alongside the Android keystore manager (§21.5)
- **Kotlin Multiplatform (KMP) awareness**: a project view distinguishing shared business logic from platform-specific code, with Leo tracking which changes to shared code need platform-specific follow-through and flagging it in the plan
- **App store submission pipeline**: Play Console and App Store Connect as Ops View deploy targets (§23) — versioning, release notes, and phased rollout status visible without leaving Spartan
- **Crash analytics integration** (Crashlytics/Sentry-style): production crashes feed into the same triage flow as local core dumps (§32.9) — one place to investigate a crash regardless of whether it happened on your machine or in production

### 37.5 Debugging

- **Async/concurrency visualizer**: for async-heavy code (Rust async, Kotlin coroutines, Go goroutines, the JS event loop), a visual timeline of task scheduling, blocking, and await points — this is precisely where traditional flat call-stack debuggers are weakest
- **Read-only production attach mode**: attaching a debug adapter to a production/remote process defaults to inspect-only — no breakpoint-triggered pause, no variable mutation — unless explicitly elevated, since pausing a live production process can itself become the incident

### 37.6 GUI Builder

- **Brand guideline enforcement**: the token panel (§34.4) can flag or hard-block color/type values falling outside an approved brand palette, configurable as strict or warn-only per project
- **Screen-reader simulation mode**: preview what a screen reader would actually announce for the current canvas state — experiencing the accessibility gap directly, not just being told about it by a checklist
- **Design spec export to PDF/shareable doc** for stakeholders without Spartan installed

### 37.7 Studio Cross-View Automation

- **Conditional automation rules across views**, expressed as simple trigger→action pairs referencing the Project Graph (§30) rather than a separate workflow engine to learn: "if all tests pass on this branch, auto-deploy to staging," "if a security scan finds a critical CVE, block the merge task"
- **Unified notification center**: one inbox aggregating CI failures, review comments, mentions, and scheduled task results across every Workspace view, with per-category quiet hours

### 37.8 Security & Supply Chain

- **Build provenance/attestation** (SLSA-style), generated automatically per release build and cryptographically signed — verifiable proof of exactly what source produced a given artifact, feeding the SBOM work in §27
- **Device/session attestation for enterprise**: verifies Spartan is running on a managed, compliant device before permitting certain privacy-scoped or destructive operations in regulated environments

### 37.9 Onboarding & Learning

- **Skill-adaptive first-run tutorial**: extends the guided real-project onboarding from §8.6 with pacing that adjusts to the person's stated experience level rather than one fixed script for everyone
- **Unified in-IDE documentation search**: official language/framework docs, man pages, and Leo-generated explanations all searchable from one command-palette entry point, cached locally so it still works offline

### 37.10 Reliability & Telemetry

- **Automatic reproducible bug reports on crash**: captures the last N actions from the append-only session log (§7) leading up to a crash and offers to attach a redacted repro sequence — a dramatically more useful bug report than a bare stack trace, and still local-first per the crash reporter policy in §18
- **Explicit offline capability matrix**: a clear published table of what works with zero connectivity (local Leo via Ollama, editor, debugger) vs. what needs network (cloud Claude, marketplace, cloud deploy) — no ambiguity about what a no-internet day looks like

### 37.11 Interface Polish (Design System Completion)

- **Consistent empty/loading/error states across every panel** — not just Design View's state preview (§34.8) — so the test explorer, dependency graph, and logcat panel all have intentionally designed states instead of a blank void on first use
- **Toast/notification system** with clear severity tiers (info/success/warning/error) and consistent placement, feeding the unified notification center (§37.7)
- **Keyboard-first navigation audit**: every mouse-driven action in every panel has a documented keyboard equivalent, checked as part of the accessibility release gate (extending §16.3)
- **Full theme parity**: every custom-rendered panel, not just the editor, is fully themed across dark/light/system-adaptive — no panel that looks unstyled or "borrowed" relative to the rest of the app

---

## 38. Open Design Integration

Confirmed against the current project: **Open Design** (open-design.ai, Apache-2.0, `github.com/nexu-io/open-design`) is a real, actively developed platform — a local-first, open-source, agent-native design layer, not a hands-on canvas tool. It sits in front of a coding agent (Claude Code, Cursor, Codex, and 20+ other CLIs), is fully BYOK with credentials never proxied through a vendor, captures brand systems as portable **DESIGN.md** files (a nine-section schema: color, typography, spacing, layout, components, motion, voice, brand, anti-patterns), ships roughly 150 ready-made design systems, and produces real runnable artifacts — HTML/CSS, decks, images, even short motion clips — rendered into a sandboxed preview and exported directly to PDF/PPTX/MP4 or handed off as real code<cite index="9-1,4-1">since screenshots, fonts, palettes, and confirmed artifacts accumulate as defaults for future sessions, and the platform ships a stdio MCP server with per-agent install scripts so any MCP-compatible agent can read a project's tokens, components, and entry HTML as a structured API</cite>.

This is a closer philosophical match to Spartan than a bolted-on Figma clone would be — Open Design's whole premise (agent generates real artifacts, brand lives as a versioned file, hand-off *is* the code) is already the same philosophy behind Leo's plan→execute→verify loop and the two-way AST sync in §6/§34. So integration means absorbing its best ideas and interoperating with its ecosystem, not just embedding a third-party app.

### 38.1 Concrete Integration

- **MCP interoperability, both directions**: Leo can register an Open Design MCP server as a tool source (extends the MCP support already noted in §3's Claude integration layer), pulling from its skill/artifact library directly inside Agent View. Conversely, Spartan exposes its own project's design tokens/components via a compatible MCP endpoint, so an external Open Design install — or any other MCP-compatible agent — can read a Spartan project's design system without anyone duplicating work.
- **DESIGN.md adopted as Spartan's canonical brand-system format** (amends §34.4): instead of a bespoke `theme.tokens.json` as the only source of truth, the token panel reads/writes a DESIGN.md file using the same nine-section schema, auto-generating the structured token JSON/Tailwind config underneath for the existing two-way canvas sync to consume. Any of the ~150 existing community DESIGN.md systems drops straight into a Spartan project and works immediately; a Spartan-authored brand file is portable back out to any other DESIGN.md-compatible tool.
- **Skills as a shared unit of design generation**: an installed skill can drive Leo's AI-assisted design generation (§34.7) with a structured, curated generation approach instead of a from-scratch prompt each time — results still land in Spartan's own reviewable artifact/diff pipeline, never bypassing approval.
- **Brand extraction as a first-class Design View action**: "drop a screenshot or Figma export, extract a DESIGN.md" reuses the sketch/wireframe-to-component vision pipeline already spec'd in §34.7, just targeting a DESIGN.md as the output artifact.
- **Sandboxed generative preview as a third canvas mode**: alongside the two-way-synced hands-on canvas, Design View gains a generative mode where Leo streams a full artifact (page, component, deck, or short motion clip) into a sandboxed iframe, editable in place, and promotable into the real two-way-synced canvas once it's close — rather than forcing one paradigm for the whole builder.

### 38.2 Reconciled Design View Architecture (amends §6, §34)

Design View now has two complementary authoring modes sharing one token/component model instead of one canvas trying to do both jobs:

| Mode | Model | Best for |
|---|---|---|
| **Hands-on Canvas** (§6.1–6.2, §34.1) | Direct manipulation, two-way AST sync, precise pixel/layout control | Refining an existing component, exact spacing/constraint work |
| **Agent-Driven Generation** (new, Open-Design pattern) | Describe intent → Leo streams a full artifact into a sandboxed preview, sourced from a DESIGN.md brand + skill library | Fast first-pass layouts, whole-page scaffolds, brand exploration |

Both write to the same component tree and the same DESIGN.md-backed token store — a page generated in Agent-Driven mode is immediately editable in Hands-on Canvas mode with no conversion step, and a manual tweak in Canvas mode is visible the next time Leo generates something in that brand. This is the direct, concrete answer to the original brief to integrate a GUI builder "like Open Design": not a canvas bolted on as an afterthought, but the actual generative, agent-native, brand-as-file model fused with the precision two-way-synced canvas this spec already built.

### 38.3 Marketplace Tie-In (amends §29)

The Template/Workflow marketplaces gain a **Design Systems** category, directly compatible with — and seedable from — the existing open DESIGN.md ecosystem. Teams don't start from zero, and a DESIGN.md authored anywhere else is a drop-in, not an import job.

### 38.4 Why This Reinforces §36's Hardening

Open Design's own positioning — local-first, BYOK, credentials never proxied, artifacts as version-controlled files rather than vendor-cloud documents — is the same trust posture Section 36's failure-mode hardening pushed Spartan toward independently (Model Integrity Guarantee, transparent pricing, no silent backend substitution). Adopting DESIGN.md and MCP interoperability isn't just a feature integration; it's consistent with the whole document's stance that design and code should be inspectable, portable, and owned by the user — never opaque, never locked to one vendor's cloud.

---

## 39. Tier 0 Spike Specifications

Detailed specs for the four risk-gate spikes introduced in §35.3. Each is written to actually hand to an engineer, with a defined time-box, explicit in/out scope, measurable success criteria, and — critically — a pivot plan for failure, since the entire point of a spike is that "no" is a valid, useful outcome.

### 39.1 Spike 0.1 — Rope + GPU Renderer Latency

| Field | Detail |
|---|---|
| **Objective** | Prove the custom Rust rope + wgpu renderer can hit <5ms p99 keystroke-to-glyph latency on a 50k-line file — this validates the entire "no Monaco/CodeMirror" bet before anything else is built on top of it |
| **Time-box** | 3–4 weeks, 2 engineers (1 data-structures/rope focus, 1 graphics/wgpu focus) |
| **In scope** | Persistent B-tree rope with edit + undo-tree snapshotting (§2.1); SDF glyph atlas rendering; damage-region re-rasterization; text shaping via `rustybuzz`/`swash`; cursor/selection rendering; an automated benchmark harness that replays recorded keystroke traces (steady typing, held-key repeat, large-block paste, rapid undo/redo, scroll-while-typing) |
| **Out of scope** | LSP, full UI chrome, tree-sitter (stub with static syntax coloring), multi-cursor, undo *UI* (the data structure only) |
| **Deliverable** | Standalone benchmark binary + minimal window editing a real 50k-line file, producing p50/p95/p99 latency numbers across the trace corpus on defined reference hardware |
| **Success criteria** | p99 <5ms input-to-photon; cold file open <100ms to first paint; rope memory overhead <20% vs. a flat buffer |
| **Failure criteria & pivot** | If p99 exceeds 8–10ms after two optimization passes: escalate to a design review. Pivot options in order of preference — (1) cache SDF reshaping more aggressively before relaxing anything else, (2) drop branching undo for a simpler linear-undo rope in v1, (3) as a last resort, revisit whether GPU-native rendering justifies the investment vs. a proven text-rendering library |
| **Exit artifact** | Written benchmark report + explicit go/no-go recommendation — this is the single highest-leverage spike in Tier 0 |

### 39.2 Spike 0.2 — LSP + DAP End-to-End (Rust Reference Language)

| Field | Detail |
|---|---|
| **Objective** | Validate the in-house LSP/DAP client architecture (§2.3) end-to-end on one real language before building five more language profiles on the same pattern |
| **Time-box** | 2–3 weeks, 1–2 engineers |
| **In scope** | Full JSON-RPC/stdio transport to `rust-analyzer`, debounced `didChange`, functional (not polished) completion/diagnostics/hover UI; one DAP adapter (LLDB) with breakpoint set/hit/step/variable-inspect against a real Rust binary; **breakpoint persistence through an edit that shifts line numbers**, proving the rope-position-anchored breakpoint design |
| **Out of scope** | Multi-language support, polished completion ranking, conditional breakpoints, watch expressions |
| **Deliverable** | A working debug session inside the Spike 0.1 shell (or a throwaway harness if 0.1 isn't ready): set a breakpoint, edit a line above it, confirm the breakpoint stays attached to the correct logical line, not the original line number |
| **Success criteria** | Completions/diagnostics round-trip <150ms typical; breakpoint survives the edit-shift test; no crashes across a 30-minute real debug session |
| **Failure criteria & pivot** | If rope-anchored breakpoint persistence proves too architecturally complex in the time-box, fall back to line-number-based breakpoints for v1 (document the limitation) rather than blocking the roadmap on it — this is a nice-to-have robustness feature, not a launch blocker |
| **Exit artifact** | Working demo + a written list of LSP/DAP protocol edge cases discovered, feeding directly into the language-profile registry design (§20.1) before it's replicated five more times |

### 39.3 Spike 0.3 — ModelProvider Dual-Backend Agent Loop

| Field | Detail |
|---|---|
| **Objective** | Validate that one tool-calling agent loop runs against both Claude (native tool use) and a local Ollama model (via the structured-output fallback, §3.4) — the single highest-uncertainty item in the entire spec |
| **Time-box** | 3 weeks, 2 engineers (1 backend/Rust, 1 agent-loop/prompt design) |
| **In scope** | `ModelProvider` trait + `ClaudeProvider` + `OllamaProvider`; a minimal real tool belt (`read_file`, `edit_file`, `run_terminal` only); full plan→execute→verify loop against a small real repo; the JSON-partial-parsing fallback state machine tested against at least two non-native-tool-calling local models (one ~7B class, one ~13B class) |
| **Out of scope** | Checkpointing/rollback, sub-agents, the memory system, the full tool belt |
| **Deliverable** | A CLI-level agent (no UI required yet) that completes a real task end-to-end against both backends — e.g., "add input validation to this function and add a test" |
| **Success criteria** | Claude path completes the task reliably across 10 trial runs; local-model fallback successfully parses tool calls on ≥80% of attempts for the target model class, with 100% of failures surfaced clearly to the user, never silently dropped (per §3.4) |
| **Failure criteria & pivot** | If local-model tool-call fidelity stays well below 80% even after fallback-parser tuning, the pivot is **not** cutting local-model support — it's shipping v1's local-model path as manual-approve-only with a single-tool-call-per-turn constraint, a real but more conservative feature, with autonomous local-model mode deferred to v2 once fidelity improves |
| **Exit artifact** | A fidelity report per tested model, directly informing the curated-model manifest (§3.3) and the go/no-go on autonomous mode for local models at launch |

### 39.4 Spike 0.4 — Native UI Skeleton + WebView Canvas Integration

| Field | Detail |
|---|---|
| **Objective** | Prove the three-column skeleton is buildable in a custom immediate-mode Rust UI, and that a WebView-based Design View canvas can share state with it without feeling like two apps — the exact Antigravity 2.0 failure mode flagged in §35.9 |
| **Time-box** | 2–3 weeks, 1–2 engineers (UI/graphics + WebView bridge) |
| **In scope** | Three-column resizable layout; mode toggle (Agent/Editor/Design placeholders) with the cross-fade transition (§8.4); one embedded WebView panel showing a trivial counter/state value synced live in both directions with the native shell (proving the bridge works, not just renders); basic artifact card rendering |
| **Out of scope** | Real editor content, real Design View functionality, real Leo integration |
| **Deliverable** | A clickable skeleton demonstrating mode switching plus native↔WebView state sync |
| **Success criteria** | Perceived mode-switch time <200ms; WebView state round-trip <50ms; no visible flash/reload on switching modes |
| **Failure criteria & pivot** | If the WebView integration introduces jank or state desync: (1) keep the WebView canvas but make mode-switch an explicit, honest loading transition rather than pretending it's seamless, or (2) investigate a native renderer for non-interactive preview modes, reserving WebView strictly for hands-on canvas editing |
| **Exit artifact** | Skeleton demo plus a written verdict specifically on the qualitative "does this feel like one app" question — this is a UX risk, not just a performance number, and should be judged as such |

---

## 40. Tier 1 (v1) Sprint-Level Backlog

### 40.1 Planning Assumptions

- 2-week sprints, roughly 7 parallel workstreams running concurrently once Tier 0 gates pass, each staffed 2–4 engineers
- Sprint blocks below are dependency-ordered, not calendar-fixed — treat "Sprint Block N" as "the Nth thing that can start once its dependencies clear," not a hard date
- Sizing: **S** = 1–3 days, **M** = ~1 sprint, **L** = 2–3 sprints, **XL** = 4+ sprints
- All workstreams assume their relevant Tier 0 spike (§39) has passed; where a spike's pivot plan changes scope, the affected epic below should be re-cut accordingly, not silently overrun

### 40.2 Workstream A — Core Engine

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| Production rope + renderer | Harden Spike 0.1's rope into a real editing component; multi-cursor; undo/redo UI; large-file virtualized scrolling | L | Spike 0.1 | 1–3 |
| tree-sitter integration | Incremental parsing pipeline; syntax highlighting theming hook; per-language grammar loading | M | Rope hardening | 2–3 |
| File tree & tabs | Virtualized file tree, multi-pane tab management, split editing | M | — | 1–2 |
| Symbol graph v1 | Background indexer (§2.4), incremental updates on save, memory budget (§36.4.8) | L | Rope hardening, tree-sitter | 3–5 |
| VFS abstraction | Local + SSH-remote file access behind one API (defer container/browser targets to v2 per §37.1) | M | File tree | 4–5 |

### 40.3 Workstream B — ModelProvider & Leo Agent Core

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| Production ModelProvider | Harden Spike 0.3's trait/providers; prompt caching for Claude; curated Ollama model manifest (start 5–10 models) | M | Spike 0.3 | 1–2 |
| Full tool belt v1 | `search_codebase`, `run_tests`, `lint_check`, `git_ops`, `browser_preview` added to the 3-tool spike set | L | Spike 0.3, Symbol graph v1 | 3–5 |
| Agent state machine | Plan→Approve→Execute→Verify→Recovering loop (§4.1), configurable approval matrix | L | Production ModelProvider | 3–5 |
| Checkpointing | Git-plumbing snapshots + non-git shadow store (§4.2) | M | Agent state machine | 5–6 |
| Single Writer Invariant | One write lock per buffer, Conflict artifact type (§36.4.1) | M | Rope hardening (A), Agent state machine | 5–6 |
| Untrusted-repo quarantine mode | New-repo detection, default-deny auto-run/secrets until trusted (§36.4.2) | S | Agent state machine | 6 |
| Plan-scope enforcement | Scope Expansion approval step for out-of-plan edits (§36.4.3) | M | Agent state machine | 6–7 |
| Path-jailing | Canonicalized, project-root-jailed path resolution at the sandbox layer (§36.4.6) | M | Tool belt v1 | 6–7 |
| Project-tier memory | `.spartan/memory/project.md`, summarization/compaction (§4.3, project tier only — skip team tier for v1) | M | Agent state machine | 7 |

### 40.4 Workstream C — Language & Debug Adapters (Launch Set: Rust, TS/JS, Python, Kotlin, Java, Go)

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| Language profile registry | `LanguageProfile`/`Task`/problem-matcher model (§20.1–20.2), auto-detection from manifest files | M | Spike 0.2 | 2–3 |
| Replicate LSP client x5 | Wire TS/JS, Python, Kotlin, Java, Go LSPs using the Spike-0.2-validated pattern | XL | Language profile registry | 3–6 |
| Replicate DAP client x5 | debugpy, Delve, JDWP (shared Kotlin/Java), Node inspector — LLDB/GDB already done in Spike 0.2 | L | Replicate LSP client x5 | 4–7 |
| Model Integrity Guarantee | Model badge structurally sourced from `ProviderId` (§36.4.5) | S | ModelProvider (B) | 3 |
| Task runner UI | Streamed task output, problem-matcher-driven diagnostics anchoring | M | Language profile registry | 3–4 |

### 40.5 Workstream D — Android First-Class

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| SDK/Gradle integration | SDK manager panel, `sdkmanager` wrapper, Gradle task discovery, Gradle-aware problem matcher | L | Language profile registry (C) | 3–5 |
| Kotlin/Compose language support | Kotlin LSP, Compose-aware completions, Compose preview subprocess with auto-restart-on-crash (§37 Xcode-lesson pattern) | L | SDK/Gradle integration | 5–7 |
| JDWP on-device debugging | JDWP DAP adapter, device panel breakpoint attach | M | Replicate DAP client x5 (C) | 6–7 |
| Device management core | AVD manager, physical device detection via `adb devices` | M | — (can start early, parallel) | 2–3 |

### 40.6 Workstream E — ADB Integration

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| Device panel tabs (Devices/Files/Shell) | Two-pane file browser (push/pull), interactive shell PTY | M | Device management core (D) | 4–5 |
| Logcat tab | Buffer/priority/tag filtering, crash-buffer→tombstone linking | M | Device panel tabs | 5 |
| Processes + Screen tabs | `ps`/`jdwp` list, screenshot, basic screen mirror (**shell out to `scrcpy` per §35.7 rather than reimplementing the protocol for v1**) | M | Device panel tabs | 5–6 |
| Package Manager tab | Install/uninstall with flag checkboxes, permission viewer | S | Device panel tabs | 6 |
| Leo ADB tool set | `adb_devices`, `adb_shell_exec`, `adb_install`/`uninstall`, `adb_logcat_query`, `adb_screenshot` — independently permissioned per §33.10 | M | Full tool belt v1 (B), Device panel tabs | 6–7 |

### 40.7 Workstream F — Interface Shell & Design System

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| Production three-column shell | Harden Spike 0.4's skeleton; named layouts, focus mode | M | Spike 0.4 | 2–3 |
| Artifact card system | Plan/Task List/Diff/Verification/Conflict card types, comment affordance (§8.5, §36.4.1) | L | Production shell, Agent state machine (B) | 3–5 |
| Command palette | Unified ⌘K: file nav, commands, natural-language Leo entry | M | Production shell | 4–5 |
| Visual design system application | Color/type/motion tokens from §8.2–8.4 applied across all shell chrome | M | Production shell | 4–6 |
| Accessibility baseline | Screen-reader tree alongside the wgpu render tree, high-contrast theme, reduce-motion setting (§16.3 — **not deferrable**) | L | Production shell | 3–6, ongoing |
| Toast/empty/error state system | Consistent states across all panels (§37.11) | M | Artifact card system | 6–7 |

### 40.8 Workstream G — GUI Builder MVP (React + Open Design Dual-Mode)

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| WebView canvas + React codegen | AST mutation via `swc`, HMR dev-server bridge, `CanvasEdit` event model (§6.1–6.2) | L | Spike 0.4 | 4–6 |
| DESIGN.md token pipeline | Nine-section schema parser/writer, token panel UI, Tailwind/CSS-var export (§34.4, §38.1) | M | WebView canvas | 6–7 |
| Agent-driven generation mode | Sandboxed artifact preview, Leo streams full-page scaffolds, promotable to hands-on canvas (§38.2) | L | WebView canvas, Full tool belt v1 (B) | 7–8 |
| Component library browser | Basic component/variant browsing, prop-to-token binding | M | DESIGN.md token pipeline | 7–8 |

### 40.9 Workstream H — Cross-Cutting Hardening (Security, Plugins, Reliability)

| Epic | Key stories | Size | Depends on | Sprint block |
|---|---|---|---|---|
| WASM plugin API core | Capability manifest model, WIT-typed host-guest interface, 2–3 reference plugins | L | Production shell (F) | 3–6 |
| Per-plugin resource budget | CPU/memory budget enforced by the WASM runtime, auto-throttle (§36.4.9) | M | WASM plugin API core | 6–7 |
| Secrets scanning pass | Redaction before any cloud-bound context assembly (§9) | S | ModelProvider (B) | 3 |
| Crash reporter | Local-first triage, redact-before-upload, reproducible bug report from session log (§18, §37.10) | M | Production shell (F) | 5–6 |
| Update channel control | Stable/beta/nightly, pinnable, no forced auto-update (§18, direct Antigravity lesson) | S | — | 2 |
| Pre-flight cost estimates | Shown in plan artifact before execution above a threshold (§36.4.4) | S | Agent state machine (B), Artifact card system (F) | 6 |

### 40.10 Cross-Workstream Integration Milestones

These are the points where independent workstreams must demonstrably work together, not just individually — historically where "feels like separate tools" risk hides:

| Milestone | What it proves | Target sprint block |
|---|---|---|
| **M1 — First agentic edit in the real shell** | Leo (Workstream B) proposes and applies a diff inside the production editor (A) with an artifact card (F) — the core loop, for real, in the real UI | 5 |
| **M2 — Debug a real bug end-to-end** | Set a breakpoint, hit it, inspect state, ask Leo to explain the stack trace, accept a fix — across A, B, C in one session | 7 |
| **M3 — Android round trip** | Edit Kotlin/Compose code, see live preview update, deploy to an emulator via ADB, view logcat, attach JDWP debugger — D + E + A + C together | 8 |
| **M4 — Design View round trip** | Generate a page in Agent-Driven mode, promote it to Hands-on Canvas, edit a token, see the change reflected in the running app | 8–9 |
| **M5 — Untrusted repo → quarantine → trust → full agent run** | The full security posture from Workstream B/H demonstrated as one coherent user journey, not isolated unit tests | 7–8 |

### 40.11 Definition of Done for v1

Directly restating §35.4's success criteria as an explicit release gate, plus the §36.5 checklist — v1 is not "done" until:

- A developer can use Spartan as a daily driver on a real Rust/TS/Python/Kotlin project
- Leo's plans, diffs, and verification results are visible artifacts for every agentic action — no invisible state changes
- Debugging works across at least one native and one Android target using the real device panel
- One framework (React) has full two-way GUI builder sync, plus the Open Design agent-driven generation mode
- Every item in the §36.5 release-gate checklist answers correctly
- Accessibility baseline (screen reader, high contrast, reduce motion) passes a real audit, not just a self-check

---

## 41. Hugging Face Model Downloader for Ollama (amends §3.3)

Section 3.3 gave the Ollama model manager a curated picker and basic pull/delete wrapping. This section extends it into a full Hugging Face-integrated downloader — confirmed against Ollama's actual current mechanics rather than assumed: Ollama can pull GGUF models directly from the Hub via a `hf.co/{user}/{repo}` reference, defaults to the `Q4_K_M` quantization when present in the repo, and supports selecting a specific quantization with a `:TAG` suffix (both `hf.co` and `huggingface.co` work as the domain, and this works for private/gated repos too, given a token).

### 41.1 Rationale

Ollama's own library is a curated slice; Hugging Face hosts the much larger long tail of community fine-tunes, quantizations, and niche coding models. A developer picking a local model for Leo shouldn't have to leave Spartan, hand-copy a repo name, and run shell commands to try it. This should be as native as the existing curated-model picker in §3.3, not a separate workflow.

### 41.2 Browse & Discover

The Model Manager panel (§3.3) gains a third tab alongside **Installed** and **Curated**: **Hugging Face**.

- Queries the Hugging Face Hub search API, filterable by task (text-generation, code), library (`gguf`), and sort (trending/downloads/recently updated) — the same facets Hugging Face itself exposes on its own [Ollama-tagged model collection](https://huggingface.co/models?apps=ollama), surfaced natively rather than requiring a browser trip
- Each result card shows: model name/repo, parameter count, available quantization variants, total downloads, license, and last-updated date
- A **"Recommended for your hardware"** badge appears on the quantization variant whose estimated memory footprint (file size + a 1–2 GB KV-cache margin, per real-world Ollama sizing guidance) best fits the system specs Spartan already detected (§3.3's open decision on hardware-aware onboarding) — surfaced as a badge, never an auto-selection the user didn't ask for
- The existing curated manifest (§3.3) is folded in as a pinned **"Editor's Picks"** row at the top of this tab, not a separate competing list

### 41.3 One-Click Pull

- Selecting a model + quantization variant issues the equivalent of `ollama pull hf.co/{user}/{repo}:{QUANT}` through Ollama's REST API, reusing the exact same streamed progress UI already built for the curated picker (§3.3) — one pull pipeline, two entry points
- If a repo doesn't expose an explicit quantization tag, Spartan surfaces Ollama's own default-selection behavior transparently (labeled "repo default — usually Q4_K_M") rather than hiding which variant actually got pulled
- Quantization variant list includes a short, plain-language tradeoff note per tier (roughly: Q4_K_M/Q5_K_M as the typical quality/size sweet spot, Q8_0 for maximum fidelity at double the footprint, Q2/Q3 tiers flagged as noticeably lossy) — informational, not prescriptive

### 41.4 Advanced Import Pipeline

For models that aren't a simple single-file GGUF pull, Spartan shells out to the same established tooling the ecosystem already relies on rather than reimplementing conversion logic — consistent with the "reuse, don't rebuild" call already made for screen mirroring in §35.7:

| Case | Handling |
|---|---|
| **Sharded GGUF** (`model-00001-of-00005.gguf` etc.) | Ollama cannot run sharded GGUF files directly. Spartan detects this pattern on download and automatically runs `llama-gguf-split --merge` before registering the model, rather than surfacing a cryptic failure — this is exactly the kind of silent-footgun case §36's hardening philosophy exists to catch |
| **Safetensors-only repo** (no GGUF published) | Offered as an explicit "Convert & Import" action, wrapping `convert_hf_to_gguf.py` from llama.cpp as a background Task (§20.2) with visible stages: Downloading → Converting → Quantizing → Registering. Never silent, always cancelable |
| **On-the-fly quantization** | For FP16/FP32 Safetensors models, exposes Ollama's own `ollama create -q <level>` quantization step as part of the same import flow, rather than requiring a separate manual pass |
| **LoRA adapters** | A distinct "Import Adapter" flow, enforcing the same base-model match Ollama itself requires — Spartan blocks (not just warns) an adapter/base-model mismatch before attempting the import, since that failure mode is confusing to debug after the fact |
| **Private/gated repos** | Hugging Face access token stored via the same OS keychain integration used for signing keys and other secrets (§21.5, §27) — never in plaintext, never sent to Leo's context |

### 41.5 Storage & Disk Management

- Pre-flight disk-space check before any pull begins — shows required space vs. available and blocks the pull with a clear message rather than failing halfway through a multi-gigabyte download
- Resumable downloads: an interrupted pull (network drop, app restart) resumes rather than restarting from zero, consistent with treating long-running operations as first-class `Task`s (§20.2) with real lifecycle state, not fire-and-forget
- A storage panel (extending §3.3's model manager) shows disk usage per installed model with one-click delete, since local GGUF files accumulate quickly at multi-gigabyte sizes

### 41.6 Leo Tool Integration

Two new tools extend Leo's belt, letting the agent participate in model selection rather than requiring the human to do it manually every time:

| Tool | Behavior |
|---|---|
| `search_hf_models` | Queries the Hub with task/size/license filters — e.g., "find me a good local coding model under 8GB" resolves to a real search, not a guess from training data |
| `pull_hf_model` | Proposes a specific model+quantization pull as part of a plan artifact (§4.1) — multi-gigabyte downloads are treated as a real action requiring the normal approval flow, not something Leo does silently mid-conversation |

### 41.7 Hardening Notes (ties to §36)

- Checksum/hash verification against the Hub's published file metadata before a model is registered, catching a corrupted or truncated download rather than letting a broken model fail confusingly at inference time
- Every pull, conversion, and quantization step writes to the same append-only session log (§7) as any other agent or user action — model provenance (which repo, which quant, when) stays fully auditable, which matters directly for the Model Integrity Guarantee (§36.4.5): the badge showing "Leo · some-model (local)" should always be traceable back to exactly what was pulled and from where

---

## 42. Settings Panel — Comprehensive Architecture

This document has accumulated a lot of configuration surface — routing modes, approval matrices, memory tiers, privacy rules, keybindings, accessibility toggles, plugin permissions, HF tokens, and more added in every pass since — scattered across the spec wherever each feature was introduced. This section unifies all of it into one real settings system, which is arguably the most direct way to "enhance every aspect" at once: every subsystem in this document gets a proper control surface here, rather than an assumed config file only a developer would know to edit. §42.2's category table is kept in lockstep with whatever settings categories actually exist in the reference prototype — the same discipline §51.1 already applied once to catch a structural bug in this exact area, now applied preemptively instead of after the fact.

### 42.1 Design Principles

- **Same transparency philosophy as the rest of Spartan** (§36): any settings change that affects security, privacy, or cost is itself logged to the audit trail (§18), and security-relevant changes get the same confirmation weight as a destructive tool action — a toggle is not a substitute for a warning where the stakes warrant one
- **Layered resolution, visibly**: System defaults → Global (user) → Project (`.spartan/config.toml`, §7) → Session override. Every setting row shows which layer actually set its current value, so "why is this different on my machine" has an immediate, visible answer instead of a debugging session
- **Fully keyboard- and screen-reader-navigable** — the settings panel is not exempt from the accessibility baseline (§16.3, §37.11); if anything it needs it most, since it's where assistive-tech-dependent users configure everything else
- **Searchable by intent, not just by label** — one search bar at the top, feeding the same index behind the command palette (§16.1), resolving natural-language queries like *"how do I stop Leo from auto-running terminal commands"* directly to the relevant approval-matrix setting

### 42.2 Settings Information Architecture

Opens as a dedicated full-pane view (swapping center stage, the same weight as a mode switch) rather than a cramped modal — a product this deep deserves real screen space for its settings, not a 400px popup.

| Category | What lives here | Ties to |
|---|---|---|
| **General** | Update channel (stable/beta/nightly, pinnable), language/locale, telemetry opt-in with an exact "what we collect" list | §18, §37.9 |
| **Appearance** | Theme (dark/light/system), accent customization, reduce-motion, UI + code font choices, named layout manager | §8.2–8.4, §16.2, §37.11 |
| **Leo & Models** | Routing mode (Cloud-only/Local-only/Hybrid/PrivacyScoped) with the per-path rule editor, curated Ollama manifest, Hugging Face downloader, **LiteLLM provider connections with fallback-chain editor (§44.5)**, per-task-type model tier selection, cost budget caps | §3, §36.4.4, §41, §44 |
| **Agent Behavior** | Approval matrix per action-class (file edit / terminal / git push / destructive ops), autonomy level — **Manual / Plan-Approve / Autonomous / Vibe (§45)**, untrusted-repo quarantine defaults and the trusted-repo list, plan-scope enforcement strictness, sub-agent concurrency limit | §4.1, §4.4, §36.4.2, §36.4.3, §45 |
| **Memory** | Project/Global/Team memory files, opened as real editable markdown — not a black box — plus local preference-learning opt-in/reset | §4.3, §37.2 |
| **Privacy & Security** | Privacy-scoped path rules editor, secrets vault backend selection, path-jailing status (read-only, always-on), plugin capability review, device/session attestation status, **external content fetch gating and .gitignore-scoped context exclusion (§50.2)** | §3.5, §9, §27, §36.4.6, §50.2 |
| **Languages & Toolchains** | Installed language profiles, SDK/toolchain paths (Android SDK/NDK, JDKs, etc.), per-language formatter/linter config, remote/containerized build delegation | §20.1, §21.1 |
| **Debugging** | Default debug adapter per language, symbol server config, crash/core-dump handling preference | §32 |
| **ADB & Devices** | Device panel defaults, wireless pairing management, destructive-ADB-action confirmation toggle | §33 |
| **Design View** | DESIGN.md brand file location, token export target, Open Design MCP connection status | §34, §38 |
| **Plugins & Extensions** | Installed WASM plugins with per-plugin resource budget and capability display, marketplace source config, sideload folder, Antigravity/VS Code extension manifest import with a per-capability conversion report | §5, §36.4.9, §68 |
| **Skills** | Installed/Import/Marketplace tabs for lightweight markdown+script capability packages, per-skill scope and enable toggle | §63 |
| **MCP Servers** | Connected-server list (transport, status, tool count), add-server flow, per-tool allowlist, health check | §64 |
| **Notifications** | Unified notification center preferences, per-category quiet hours, toast severity rules | §37.7, §37.11 |
| **Keybindings** | Full keymap editor, preset switcher (Spartan / VS Code / JetBrains / Vim / Emacs), per-command rebinding, exportable keymap file | §19, §37.11 |
| **Accessibility** | Screen-reader verbosity, high-contrast, reduce-motion, minimum tap-target size, focus-outline style | §16.3 |
| **Enterprise & Org** *(visible only when active)* | SSO config, org policy engine rules (read-only for non-admins), centralized audit export | §28 |
| **CLI & Remote** | Companion-mode pairing status with the desktop daemon, standalone/headless config for CI and remote hosts, non-interactive approval policy file location, shell completion install | §46 |
| **External Agent Fleet** | Registered third-party CLI engines with detection status and fallback chains, registry file location, auto-switch-on-quota policy, Fleet Health self-check timestamps | §52 |
| **Import & Migration** | Per-project detection of another AI tool's config in the open workspace, per-preference-category import (instructions/rules, MCP servers, keybindings, theme, model preference) with a per-item conversion report — never a silent approval-policy import | §70 |
| **IoT & Embedded** | Detected board registry (toolchain, RTOS), serial monitor baud default, MQTT Inspector broker, OTA update toggle explicitly labeled network-capable | §72 |
| **Security Auditor** | Verified-vs-flagged findings list with severity, a two-step confirm-then-run exploit-verification trigger scoped to the open project only, per-finding proposed-fix handoff to Leo | §73 |
| **Decompiler** | Engine registry (Ghidra default, radare2/JADX/CFR/ILSpy detection status), a two-step confirm-then-decompile flow treating any non-project binary as untrusted content, read-only pseudocode view | §74 |
| **Advanced** | Raw `config.toml` editor for anything without a dedicated UI yet, experimental feature flags, diagnostic log level, Neural Link and Ops Cockpit companion toggles — explicitly labeled as power-user territory | catch-all |

**This table has gone stale three separate times** — Skills/MCP/Fleet, then IoT/Security Auditor/Decompiler, each caught only on a later audit pass rather than when the category was actually added. The standing rule going forward: adding a category to `SETTINGS_CATEGORIES` in `interface-prototype.jsx` and this table are **one edit, not two sequenced ones** — do both in the same pass that adds the category, not as a follow-up. A release audit is the wrong time to be finding this for the first time; it should never have gone stale to find.

### 42.3 Cross-Cutting Settings-System Features

- **Presets/Profiles**: one-click named bundles for common postures — *Solo/Fast* (autonomous approval, hybrid routing, minimal confirmations), *Team/Careful* (plan-approve-every-step, cost caps, strict quarantine), *Air-Gapped* (local-only routing, no telemetry, no marketplace network calls). Switching a preset shows a diff of exactly what will change before applying — never a silent bulk overwrite, consistent with the Single Writer/no-silent-change philosophy running through this whole document
- **Settings-as-code**: the full resolved settings tree exports to a version-controllable file extending `.spartan/config.toml` (§7) — a team reviews a settings change the same way they review a code change, and project-level settings committed to the repo are diffable in git like anything else
- **Change history**: every settings change is timestamped and one-click reversible — settings aren't a special case exempt from "everything is undoable" (§4.2's checkpointing philosophy applies here too)
- **Security-relevant change confirmation**: widening auto-approval scope, disabling quarantine mode, or adding a PrivacyScoped exception requires an explicit confirmed action, not a casual toggle-and-forget — mirroring the destructive-action gating already established for agent tool calls (§4.5, §9)
- **Opt-in sync**: global (non-project) settings can optionally sync across a user's machines via an encrypted, user-owned store — never required, always visibly indicated when active, never silently on

### 42.4 Extensibility & Diagnostics

- **Plugin-contributed settings pages**: a WASM plugin can register its own settings section under Plugins & Extensions using the same capability model as everything else (§5.3's `editor_api` extension point gains a `settings.register_page` hook) — third-party tools get first-class settings, not an awkward separate config file the user has to find
- **Built-in "Doctor" diagnostics**: under Advanced, a one-click environment health check — toolchain versions, Ollama reachability, GPU capability, disk space, LSP/DAP server status — surfaced as a pass/fail list with direct links to the relevant setting for anything failing, rather than leaving the user to piece together what's wrong from scattered error messages

---

## 43. New Feature Proposals

Forty-two sections have covered nearly every pillar in real depth — another blanket sweep at this point would mostly restate what's already here. More useful now: a curated set of genuinely new ideas not yet in the spec, each tied back to the architecture it would extend rather than floating free of it.

### 43.1 Agent & Workflow

- **Experiment Mode**: fork the current working state into an ephemeral, isolated sandbox to try a risky approach without touching the real branch — extends the checkpointing model (§4.2) from "one timeline you can rewind" to "multiple timelines you can compare side by side," with a clear UI for diffing two experiments against each other before picking one and discarding the rest
- **Automated Implementation Bake-off**: takes the Multi-Option Proposal mode (§37.2) a step further — instead of just presenting written trade-offs, Leo actually implements 2–3 candidate approaches in parallel sandboxes (reusing Experiment Mode above), runs the real test/benchmark suite against each, and presents measured results alongside the trade-off writeup. Opinions become data where the cost of generating that data is low
- **Code Archaeology Mode**: "why is this written this way" walks blame history, linked tickets, and old PR discussions via the Project Graph (§30) to reconstruct the original reasoning — distinct from the "explain this codebase" onboarding mode (§19), which explains current structure; this one excavates historical intent, which matters most on legacy code nobody currently on the team wrote
- **Flow Sessions**: a time-boxed focus mode extending §16.2's Focus Mode and §37.7's notification center — non-urgent notifications are held and batched for the session's end, and if Leo would normally interrupt with a clarifying question mid-task, it queues the question and makes a reasonable documented assumption instead, flagging it for review when the session ends rather than breaking flow

### 43.2 Modern Web & API Development

- **Contract-First Development**: define an OpenAPI or GraphQL schema first; Spartan generates typed client and server stubs across every language in the project simultaneously (leaning on the multi-language registry, §20.1), and the Project Graph (§30) keeps schema, generated code, and hand-written business logic linked — a schema change surfaces every call site across every language that needs updating, not just the ones in the language you happened to edit
- **Feature Flag Management Panel**: a lightweight built-in flag system (or an integration point for LaunchDarkly-style external ones) as an Ops View panel (§23), with flags linked into the Project Graph so Leo knows which code paths a given flag gates — genuinely useful when debugging "why does this behave differently in staging," since the answer is often a flag state, not a code difference
- **Edge/SSR Request-Lifecycle Debugging**: one unified trace spanning build-time, edge-function execution, and client hydration for modern web frameworks — extends the distributed-trace waterfall (§32.7) down into the specific, currently-awkward-to-debug boundary between server-rendered and client-hydrated code

### 43.3 Knowledge, Communication & Sustainability

- **Terminal Session Recording & Sharing**: asciinema-style recording of a terminal session, shareable as a link or embedded in a PR/ticket — extends the reproducible-bug-report work (§37.10) from "captured automatically on crash" to "capturable on demand for knowledge sharing," useful for anything reproducible but hard to describe in words
- **Business-Impact Translator**: auto-generates a plain-language summary of a technical change from its plan artifact, for the Manage View's standup digest (§26) or for sharing with non-engineering stakeholders — translates "refactored auth to JWT" into what that actually means for users/uptime/security, without the person having to write two versions of the same update
- **Tech-Debt Interest Dashboard**: combines churn frequency, cyclomatic complexity, and test coverage (all already tracked individually across §14, §24, §37) into one prioritized view of which code is accumulating the most "interest" — turns "what should we refactor" from a gut-feel argument into something backed by the data Spartan is already collecting anyway
- **Energy/Carbon Footprint Tracking**: surfaces estimated energy draw for local model inference (GPU power draw during an Ollama session) alongside the existing cost dashboard (§18) — genuinely novel among competitors, and directly relevant to Spartan's own hybrid-routing pitch, since "run this locally to save cost" and "run this locally to reduce footprint" are the same decision from two angles

### 43.4 Accessibility & Inclusivity

- **Full Voice-Driven Coding Session**: extends the voice annotation feature (§15) into genuinely hands-free operation — navigation, dictation, and command execution by voice, not just leaving a note for later. A real accessibility feature for motor-impairment scenarios, not a novelty
- **Localization/i18n Testing Tooling**: pseudo-localization preview (expands strings to catch hardcoded-width assumptions), missing-translation-key detection, and an RTL mirror-test view in Design View — extends the i18n scaffolding already committed to in §16.3 from "the app can be localized" to "here's active tooling that catches localization bugs before they ship"

### 43.5 What's Actually Worth Building First

Applying the same discipline as §35 rather than treating all thirteen as equally urgent: **Code Archaeology Mode** and **Feature Flag Management** are the highest-leverage of this batch — both are cheap relative to infrastructure already built (Project Graph, Ops View) and solve real, frequent daily friction rather than a rare scenario. **Experiment Mode** is the most architecturally interesting but should wait until the core checkpointing system (§4.2) has been proven solid in production, since it's a direct extension of it. Everything else in this section is reasonable Tier 2/3 backlog material (§35), not a Tier 1 addition.

---

## 44. LiteLLM Integration (amends §3.1–3.3, §28.2)

### 44.1 Why LiteLLM

`ModelProvider` currently has exactly two concrete implementations: `ClaudeProvider` and `OllamaProvider` (§3.2–3.3). Every additional cloud provider — OpenAI, Azure OpenAI, AWS Bedrock, Google Vertex, Cohere, Mistral, Groq, Together, Replicate, and dozens more — would otherwise mean writing and maintaining a bespoke adapter per vendor API. LiteLLM already solves exactly this problem: a mature, widely-adopted open-source unification layer exposing 100+ providers behind one OpenAI-compatible interface, with built-in cost tracking, caching, retries/fallbacks, and load balancing. Rather than replacing the two existing providers — which stay as the most-optimized, first-class paths (full prompt caching for Claude, native GGUF handling for Ollama) — Spartan adds a third implementation, `LiteLLMProvider`, covering everything else through one integration instead of dozens.

### 44.2 Architecture

- Spartan manages a local LiteLLM proxy process, supervised the same way Ollama is treated as a local service (§3.3): `litellm --config .spartan/litellm.config.yaml`, bound to localhost only by default
- `LiteLLMProvider: ModelProvider` talks to this local proxy over its OpenAI-compatible REST API; the proxy fans out to whichever upstream providers are configured
- Per-provider API keys live in the generated config only as *references* — actual secrets resolve from the OS keychain/secrets vault (§27) at proxy startup, never written in plaintext, consistent with every other credential in this spec

### 44.3 Model Integrity, Preserved Through the Proxy

LiteLLM's response payload always identifies which underlying model actually served a request. The Model Integrity Guarantee (§36.4.5) extends through the proxy layer explicitly: the badge shown is never just "LiteLLM" — it's the real resolved model (*"Leo · GPT-4.1 via LiteLLM"*, *"Leo · Bedrock Claude 3.5 Sonnet via LiteLLM"*), sourced from the proxy's actual response metadata, closing exactly the gap behind Windsurf's documented model-substitution failure (§36.2), which becomes a live risk the moment any routing layer sits between the user and the model. LiteLLM's automatic fallback chains (e.g., Claude rate-limited → fall back to GPT-4) are supported but constrained: a fallback can only route to a provider that already satisfies the session's active routing/privacy policy (§3.5) — a PrivacyScoped rule forcing local-only execution is never silently bypassed by a cloud fallback just because the primary provider hit a rate limit.

### 44.4 What LiteLLM's Built-Ins Buy for Free

- **Cost tracking & budgets** feed directly into the existing cost dashboard (§18) and pre-flight estimate pattern (§36.4.4) across every provider it fronts, not just Claude
- **Caching** (exact-match and semantic) cuts redundant spend on repeated prompts
- **Load balancing** across multiple keys/deployments directly powers the Enterprise self-hosted model gateway (§28.2), which now amends to *be* LiteLLM Proxy, centrally deployed, fronting Claude and an internal Ollama fleet — rather than a bespoke Spartan-only gateway reinventing infrastructure that already exists and is battle-tested

### 44.5 Model Manager Integration (amends §3.3, §41)

The Model Manager gains a third source tab alongside Ollama and Hugging Face: **LiteLLM Providers** — add any supported backend through a guided form (provider type, endpoint, key), health-checked before it's added to the picker. Every LiteLLM-routed model appears in the same unified picker as native Claude/Ollama options, tagged with its real provider and full fallback path; a Settings → Leo & Models sub-panel (§42.2) lets you view and edit fallback chains directly, subject to §44.3's routing-policy constraint.

### 44.6 Virtual Keys (Org Deployment Hardening)

In Remote/org gateway mode, individual developer machines should hold only a scoped, budget-capped, revocable LiteLLM virtual key — never the actual underlying OpenAI/Azure/Bedrock root credentials. If a developer's machine is compromised, the blast radius is one revocable virtual key with a spend cap, not a root API key with unlimited org-wide billing exposure. This is a genuine security upgrade over per-developer raw credentials, not just a convenience layer.

---

## 45. Vibe Coding Mode (amends §4.1, §42.2)

### 45.1 Designed to Fit the Trust Model, Not Bypass It

Describing intent in natural language and letting the agent iterate rapidly without reviewing every intermediate step is now a real, common way developers want to work — especially for prototypes and exploratory code. The temptation is to build this as "turn off the safety rails," but that directly contradicts everything hardened in §36. Vibe Mode instead changes **review cadence**, not the underlying **security boundary** — it's fast because rollback is cheap and trustworthy, not because oversight is gone.

### 45.2 What Changes vs. What Stays Fixed

| Changes in Vibe Mode | Stays fixed regardless of mode |
|---|---|
| No per-diff approval prompts — Leo iterates continuously | Every change still creates a checkpoint (§4.2), always one-click revertible |
| Compact "vibe stream" feed instead of full plan/diff artifacts per step | Path-jailing (§36.4.6) still applies — no tool call resolves outside the project root |
| Periodic natural pause points (every N changes or M minutes) with a consolidated summary | Untrusted-repo quarantine (§36.4.2) still applies on unfamiliar repos regardless of mode |
| A single persistent "stop and review" control, always one click away | Full diff history stays reconstructable afterward — nothing is silently discarded, only deferred |

### 45.3 UI Treatment

A distinct, unmistakable visual mode — a specific accent treatment and a persistent "Vibe" tag in the top rail (extending §8's mode-indicator pattern), so it's never ambiguous whether careful-review or fast-iteration mode is active, mirroring the same never-ambiguous principle behind the Model Integrity Guarantee. An optional calmer visual theme (softer motion, muted palette) for Vibe Mode specifically is a genuinely nice-to-have — listed here as polish, not a commitment.

### 45.4 Guardrails on When to Suggest It

Spartan nudges — never blocks — toward Plan-Approve mode when a repo already has CI/CD or deploy targets configured in Ops View (§23); vibe coding fits a personal prototype far better than a production service, and the product should say so without refusing to allow it either way. Switching into Vibe Mode on a previously Plan-Approve project surfaces a one-time, dismissible reminder of what changed, per the settings change-confirmation pattern (§42.3) — a heads-up, not a hard gate.

### 45.5 "Formalize This Session"

A one-click action that retroactively runs the full plan-scope/diff-review pipeline over everything a Vibe Mode session did — generating the plan artifact, per-file diffs, and verification results that were skipped in the moment. This is what makes it responsible to hand a vibe-coded prototype off to a team or promote it toward production: the review artifacts aren't lost, they're just deferred until someone actually wants them.

---

## 46. Spartan CLI — Terminal Support

### 46.1 Why a CLI, Not Just an Embedded Terminal Panel

The embedded terminal panel (§14) runs commands inside the desktop app. A real CLI companion — a `spartan` binary usable from any terminal, over SSH, in CI, or on a headless server with no desktop app running at all — is a different, necessary surface: the same relationship a standalone coding-agent CLI has to a desktop assistant. This isn't a second agent; it's the same Leo engine on a second surface.

### 46.2 Two Operating Modes

- **Companion mode**: the `spartan` binary detects and connects to a running desktop instance's local agent daemon over a local socket — a CLI-initiated task shares the exact same session store, Project Graph (§30), and memory tiers (§4.3) as the desktop app, and shows up live in the desktop's session rail. One Leo, two surfaces, never a fork.
- **Standalone mode**: when no desktop instance is reachable — a remote server, a container, CI — the same core agent engine runs as a lean headless binary with no rendering dependencies at all, directly reusing the ModelProvider/agent-core layer (§3–§4), which was already architected without any UI dependency

### 46.3 Command Surface

| Command | Purpose |
|---|---|
| `spartan chat "..."` | One-shot or interactive Q&A, no file writes |
| `spartan run "<task>"` | Full plan→execute→verify agentic task |
| `spartan vibe "<task>"` | Runs in Vibe Mode (§45) from the terminal |
| `spartan plan` / `diff` / `approve` | Inspect and act on the current session's artifacts without a GUI |
| `spartan status` | List active/recent sessions, mirroring the left rail |
| `spartan config get/set` | Mirrors the Settings Panel (§42) for headless environments |
| `spartan mcp serve` | Exposes the current project as an MCP endpoint (ties to §38) |
| `... \| spartan explain` | Pipe any command's output in — logs, stack traces, diffs — for a direct explanation |

### 46.4 Scripting & CI Support

`--json` output on every command, exit codes mapped to verification results (0 = verified pass, non-zero = failure) — `spartan run --task="fix failing test" --yes --json` is a real, usable CI step, not a demo of the interactive tool. The trust model is **not relaxed for convenience** headlessly: quarantine mode, path-jailing, and plan-scope enforcement (§36.4) all still apply in CI. Non-interactive approval requires an explicit `--yes` flag or a checked-in policy file (§42.3's settings-as-code) — there is no silent default-allow just because no human is watching the terminal.

### 46.5 Shell Integration

Bash/zsh/fish completions, plus a lightweight `spartan explain-last` hook capturing the most recent command and its output/exit code for immediate explanation — fitting into an existing terminal workflow rather than asking developers to change how they already work. Over SSH into a remote or containerized dev environment (§37.1's VFS abstraction), the CLI is the primary way to reach Leo when the full GPU-rendered desktop app isn't running there at all — same agent, same Project Graph, just a lighter surface.

### 46.6 Auth on Headless Machines

On the same machine as the desktop app, the CLI reuses existing OS-keychain-stored credentials (§36.4.6's posture, unchanged) — no separate login step. On a separate or headless machine, `spartan login` runs a device-code flow (open a URL, enter a short code) — the same pattern tools like `gh` and `flyctl` already use, chosen specifically because it doesn't require a local browser or copy-pasting a raw API key into a remote shell's history.

---

## 47. Tier 0 Execution Log — What Was Actually Run

Section 39 specified four spikes. This sandbox has `rustc`/`cargo` but no GPU, no display server, and no reachable model backends (no API key available to this tool, no Ollama installed, and its installer domain isn't network-reachable here). Rather than simulate results, here's exactly what could and couldn't be executed for real, and what came back.

### 47.1 Spike 0.1 (partial) — Rope Data Structure, CPU Layer Only

Built a real Cargo project using `ropey` as the foundation (per §2.1's stated approach), benchmarked against a naive flat-`String` baseline on a synthetic 50,000-line, 3.53MB file, on this sandbox's single CPU core, release build:

```
-- Typing (single-char insert, 2000 ops) --
  rope       p50=0.0008ms  p95=0.0023ms  p99=0.0032ms  max=0.0349ms
  flat String p50=0.0496ms  p95=0.1200ms  p99=0.1533ms  max=1.6879ms

-- Large paste (2KB block, 200 ops) --
  rope       p50=0.0034ms  p95=0.0046ms  p99=0.0087ms  max=6.8377ms

-- Snapshot / clone (branching-undo proxy, 500 ops) --
  rope.clone() p50=0.0000ms  p95=0.0000ms  p99=0.0002ms  max=0.0003ms

rope p99 = 0.0032ms vs flat-buffer p99 = 0.1533ms (47.4x)
```

**What this confirms**: the rope approach is ~47x faster than a flat buffer at p99 on real insert operations, and — the more load-bearing result — `clone()` is essentially free (~0.0002ms p99), confirming `ropey`'s Arc-based structural sharing actually holds, which is what §2.1's branching-undo design depends on. The data-structure layer leaves enormous headroom under the 5ms full-pipeline target.

**What this does not confirm**: nothing about glyph rendering, damage-region rasterization, or true input-to-photon latency — that's the harder, GPU-bound half of Spike 0.1, and it requires a real display and GPU that this sandbox doesn't have. **This spike is not closed.** Only its lower-risk half is.

### 47.2 Spike 0.3 (partial) — Fallback Tool-Call Parser

This was flagged in §39.3 as the single highest-uncertainty item in the whole spec. Built the actual streaming parser state machine from §3.4 and ran it against eight adversarial synthetic token streams — not real model output, but shaped like the specific failure modes a weak local model would actually produce:

```
running 8 tests
test fence_marker_split_across_chunk_boundary_still_detected ... ok
test malformed_json_is_surfaced_not_dropped ... ok
test missing_tool_field_is_surfaced_not_silently_guessed ... ok
test multiple_tool_calls_in_one_response_both_captured ... ok
test plain_text_no_tool_call_passes_through_untouched ... ok
test runaway_unclosed_fence_does_not_hang_or_oom ... ok
test truncated_stream_mid_fence_is_surfaced_not_hung ... ok
test well_formed_tool_call_streamed_token_by_token ... ok

test result: ok. 8 passed; 0 failed
```

The two that matter most: a fence marker split across a chunk boundary (the most common real streaming failure mode) still gets detected correctly, and a stream that dies mid-fence is surfaced as an explicit failure rather than hanging or being silently dropped — the exact non-negotiable requirement §3.4 sets.

**What this confirms**: the parser's state machine logic is correct against known failure shapes and honors the "never silently drop" requirement.

**What this does not confirm**: the actual ≥80% real-world tool-call fidelity target against a real 7B/13B local model — that requires an actual Ollama instance and a GPU/enough RAM to run one, neither available here. The parser is provably correct on cases it *sees*; whether real local models format tool calls in ways this parser recognizes is still untested.

**Rechecked in a much later session, same discipline as §47.9's GPU-assumption recheck**: the earlier "GPU/enough RAM to run one, neither available here" framing bundled two separate blockers that turned out to have different, and now different-again, statuses. The GPU half is resolved (§47.9). The *network* half of the old blocker — this environment's own prior finding that `ollama.com/install.sh` returned a 403 from egress policy — is also now resolved: a real check found `ollama.com/install.sh` redirecting successfully (307) to GitHub, and the actual Windows installer (`OllamaSetup.exe`, v0.31.1, a real 1,427,919,576-byte file) downloadable with a normal 200 response. Neither was assumed; both were checked with real HTTP requests. **What is not resolved**: disk space. This machine has 6.5GB free on a 98%-full disk at the time of this check — nowhere near enough for the Ollama app plus even one quantized 7B-class model (typically 4GB+), let alone the 13B-class model §39.3 also calls for. Spike 0.3's real-model fidelity test remains genuinely blocked, but by a different, new, and honestly-stated constraint than the one originally recorded — not by a stale copy-forward of an outdated assumption.

### 47.3 Spike 0.4 — Not Executable in This Sandbox

- **0.4 (UI shell + WebView bridge)** needs a display server and a GPU-capable windowing environment to render anything at all — this sandbox has neither, and the entire point of this spike is a qualitative rendering/feel judgment that can't be evaluated from log output

Spike 0.2 was originally listed here too, as also-not-executable — that turned out to be wrong for half of it, and staying wrong in the document after it was disproven would be exactly the kind of stale claim this section exists to prevent. See §47.5.

### 47.4 Honest Bottom Line (superseded in part by §47.5 and §47.6 — kept for the historical record)

Two of four Tier 0 spikes got real, partial execution with genuine results; two are architecturally sound on paper but unverifiable without different hardware. The de-risked pieces — rope performance and snapshot cost, and the fallback parser's failure-mode handling — were exactly the parts most likely to be wrong on paper and cheapest to check early, which is what a spike is for. The rest of §39's gate still stands: 0.1's GPU half, all of 0.2, real-model fidelity for 0.3, and all of 0.4 need to run on a machine with a GPU, a display, and network access to a real model backend before Tier 1 begins in earnest.

Both projects are provided as real, runnable code — not pseudocode — under `spikes/` alongside this document.

### 47.5 Spike 0.2 (DAP half) — Actually Executed, in a Later Session

The claim in §47.3 that spike 0.2 "needs a real `LLDB` process... none installed" was true of the sandbox that wrote it. It was not re-checked before repeating in later summaries — exactly the kind of unverified carry-forward claim §36 warns against elsewhere in this document, just caught here instead of shipped. A later session checked again rather than trusting the earlier note, and found `lldb` (18.1.3) and, critically, `lldb-dap-18` — the real DAP server binary for LLDB — both present. At the time this subsection was first written, `rust-analyzer` was only a rustup proxy stub that errored on invocation, so the DAP half was executed here while the LSP half was not; §47.6 documents that gap closing minutes later in the same pass, once `rustup component add rust-analyzer` turned out to actually work rather than being assumed unavailable a second time.

**What was built**: `spikes/dap-spike`, a real Cargo crate — an in-house DAP client (Content-Length-framed JSON over a subprocess's stdio, exactly §2.3's stated approach, no third-party DAP crate) that spawns `lldb-dap-18`, and a rope-anchored breakpoint model (byte-offset anchor in a real `ropey::Rope`, shifted by an edit's insertion length, converted back to a line number) — not raw arithmetic standing in for the real dependency.

**Protocol behavior was probed manually first** (a throwaway Python script driving raw DAP messages) before writing a line of the Rust client, specifically to avoid encoding a guessed protocol sequence into "tested" code — the same "run it, don't reason your way to should work" rule this document asks of every implementer applies to writing the document's own spike code.

**Three tests, run repeatedly, not once**:

1. `breakpoint_hits_and_reports_correct_local_variable` — compiles a real two-function Rust program with `rustc -g`, sets a breakpoint via DAP `setBreakpoints`, confirms the adapter reports `verified: true` against real debug info, confirms the `stopped` event fires with `reason: "breakpoint"`, then walks `stackTrace` → `scopes` → `variables` and confirms the live local `x` really is `21` at the moment of the hit. First run failed a hand-written assertion about the stack frame's name format (`"module\`function"`, guessed rather than observed) — real lldb-dap reports a mangled symbol with a hash suffix instead. Fixed to check containment, not an exact format, and documented in the test comment rather than silently loosening the check without saying why.
2. `breakpoint_survives_an_edit_that_shifts_line_numbers` — this is §39.2's actual stated success criterion, executed literally rather than argued for: insert three new lines above `fn compute`, recompute the breakpoint's line via the rope-anchor shift (not just "it should probably still work"), independently verify that computed line against a plain string search of the edited source (so the rope math can't just be checked against itself), recompile, and confirm the real debugger stops at the recomputed line with the same local variable state as before the edit. It does.
3. `client_survives_adapter_crash_on_shutdown` — see the finding immediately below; a regression test that the client's shutdown path tolerates it.

**A real bug, found by running it, not by reasoning about it**: this `lldb-dap` build (LLVM 18.1, Ubuntu 24.04 package) reliably SIGABRTs (`free(): invalid pointer`, glibc detecting heap corruption) somewhere in its own exit path immediately after replying to a `disconnect` request — reproduced with a live debuggee, with an already-exited debuggee, and with no launch at all (bare `initialize` → `disconnect`). This is not a Spartan bug and not fixable from the client side; it's a real defect in this specific adapter build, discovered only because the shutdown path was actually exercised instead of assumed to work like every other request in the sequence did. The client's `shutdown()` treats this as an expected hazard: send `disconnect`, wait briefly for a response, then unconditionally reap the child process rather than trusting it to exit cleanly — the same "never trust a subprocess's own shutdown" discipline §4.5 already states for Spartan's own sandboxed terminal tool, now validated against a concrete case where a well-established, widely-shipped tool actually needs exactly that defense.

**What this confirms**: the in-house DAP client architecture from §2.3 works end-to-end against a real, unmodified debug adapter, including the specific rope-anchored breakpoint-persistence design that was previously only a design claim; the "never trust a subprocess's shutdown" posture from §4.5 is not theoretical caution, it is required for at least one adapter in ordinary use today.

**What this still does not confirm**: any language other than Rust, any DAP feature beyond a single unconditional line breakpoint (conditional breakpoints, watch expressions, multi-thread stepping are explicitly out of scope per §39.2 and untested here), and nothing about the UI layer that would eventually surface any of this to a user — that still depends on spike 0.4, still blocked on the same missing display/GPU as before. The "LSP half not confirmed" gap named in the version of this paragraph written a few minutes earlier lasted exactly that long — §47.6 closes it in the same pass, which is the point of re-checking assumptions instead of repeating them.

### 47.6 Spike 0.2 (LSP half) — Also Actually Executed, Same Pass

`rust-analyzer` was a rustup proxy stub earlier in this document (§47.3, §47.5) — invoking it errored with "Unknown binary." Rather than let that stand as the final word, `rustup component add rust-analyzer` was tried anyway, since a stub proxy erroring is a different fact than "not installable here," and the two had not actually been distinguished before. It downloaded and installed a real, working `rust-analyzer 1.94.1` binary. Same discipline as §47.5: don't repeat an old finding without re-checking it first.

**What was built**: `spikes/lsp-spike`, a real Cargo crate — an in-house LSP client (real JSON-RPC 2.0, Content-Length-framed, over a subprocess's stdio, no third-party LSP crate, per §2.3) plus `DidChangeDebouncer`, a standalone implementation of §2.3's "own request queue with debounced didChange (150ms idle default)" claim as an actual scheduling policy rather than only a sentence.

**Protocol behavior was probed manually first**, same as the DAP client: a throwaway Python script drove real `initialize`/`didOpen`/`textDocument/completion`/`textDocument/hover` messages against `rust-analyzer` pointed at a tiny real Cargo crate (a `Point` struct with a method, plus one deliberate type error) before any Rust client code was written.

**Five tests, run repeatedly, not once**, all against a real `rust-analyzer` indexing a real Cargo crate written fresh into a tmp dir per test (not a static fixture checked into the repo — a second `Cargo.toml` nested inside a workspace member's directory tree causes cargo to treat it as an orphaned package, so it's generated at runtime the same way `dap-spike` generates its compiled binaries):

1. `diagnostics_report_a_real_type_error_at_the_right_line` — opens a file with a real mismatched-types bug (`let bad: i32 = "not a number";`), waits past rust-analyzer's initial empty diagnostics publish for the real analysis pass, and confirms a genuine `E0308` diagnostic lands at the correct line.
2. `completion_returns_a_real_method_with_its_real_signature` — real completion request at a method-call site returns `magnitude_squared` with `detail: "fn(&self) -> i32"` — the server's actual inferred signature, not a guessed string.
3. `hover_returns_real_type_and_signature_info` — real hover response contains both the receiver type (`Point`) and the method signature, sourced from rust-analyzer's own type inference.
4. `client_survives_shutdown_cleanly` — unlike the `lldb-dap` finding in §47.5, rust-analyzer's `shutdown`/`exit` sequence was observed to work cleanly; the bounded kill-fallback is kept anyway as the same defensive posture from §4.5, not because a bug was found here.
5. `debounced_did_change_coalesces_a_burst_of_edits_into_one_dispatch` — no server involved; a pure wall-clock scheduling test proving a burst of rapid edits inside the debounce window dispatches exactly once, and that a fresh edit after quiescence starts a new cycle. Deterministic given the timing margins used, not a flaky sleep-and-hope test.

**A real, minor protocol finding along the way**: rust-analyzer's `shutdown` request rejects an empty-object `params: {}` with `"invalid type: map, expected unit"` — the request needs `params` omitted entirely, not sent as `{}`. Caught by the manual probe before it became a bug baked into "tested" client code; `LspClient::request` accepts `Option<Value>` specifically so callers can omit `params` rather than every caller improvising empty objects.

**What this confirms**: both the LSP and DAP halves of Tier 0 Spike 0.2 (§39.2) now have real, repeatable, executed evidence behind them — real diagnostics, real completions, real hover, real breakpoint hit-and-inspect, real breakpoint persistence through a line-shifting edit, all against unmodified, off-the-shelf tools (`rust-analyzer`, `lldb-dap`), not mocks standing in for either. Spike 0.2 as a whole is no longer an open Tier 0 gate in the sense §35.3/§35.9 describe it — the specific in-house-protocol-client bet it was meant to validate has now actually been exercised end-to-end on one reference language, which was the entire point of doing it on Rust first before replicating the pattern across the other ~39 language profiles in §20.1's registry.

**What this still does not confirm (at the time this subsection was written)**: any language other than Rust — see §47.7, written immediately after, for that gap closing partially in the same pass — polished completion ranking or UI presentation of any of this, incremental (as opposed to full-document) `didChange` sync against a real server, and — same as always — nothing about spike 0.1's GPU half or spike 0.4, both still blocked on the same missing display/GPU this document has been honest about since §47.3.

**A flake, observed once, not fully root-caused — reported rather than hidden**: `cargo test --workspace --release` failed once in roughly ten repeated full-workspace runs, in `lsp-spike`'s test binary; the same binary run in isolation (`cargo test -p lsp-spike`) passed every time it was tried, including immediately after the failure. The likely cause is resource contention on this sandbox's 4 CPU cores when `dap-spike` (spawning `lldb-dap` + `rustc`) and `lsp-spike` (spawning `rust-analyzer`, which does real project indexing) execute concurrently as cargo runs each workspace member's test binary in parallel — but that is a plausible explanation, not a confirmed root cause, since the one failure wasn't captured with full diagnostic output before it stopped reproducing. Recording this honestly rather than quietly re-running until green: **the LSP/DAP spikes are stable when run per-crate, and not yet proven stable under full-workspace parallel execution on constrained hardware.** A real fix (raising `INDEXING_TIMEOUT`, or serializing subprocess-heavy tests via `--test-threads=1`, or both) should be verified against a reproduction before being claimed, not applied speculatively and assumed to have worked.

### 47.7 The Registry-Replication Risk — Tested on a Second Language, Same Session

§47.6 named "any language other than Rust" as the one thing it still didn't confirm, and specifically flagged this as the exact risk §40.2's backlog exists to de-risk: "Replicate LSP client x5... using the Spike-0.2-validated pattern." Rather than leave that as an open question for a future Tier 1 sprint, it was checked immediately: this sandbox also has a working `pyright` (LSP) and `debugpy` (DAP) install for Python (`pip install pyright debugpy`, both succeeded — network access to PyPI is allow-listed the same way crates.io is). So the exact same `DapClient` and `LspClient` from `dap-spike`/`lsp-spike` — not new clients, not a new abstraction — were pointed at Python instead of Rust.

**The LSP side worked with zero client changes.** `same_lsp_client_gets_real_diagnostics_completion_and_hover_from_pyright` reuses `open_project`, `wait_real_diagnostics`, `completion`, and `hover` unmodified against `pyright-langserver`, and gets a real `reportAssignmentType` diagnostic, a real `magnitude_squared` completion item, and real hover text — the only new code needed was `spawn_with_args`, an additive method for servers that need a stdio flag (`rust-analyzer` needs none; `pyright-langserver` needs `--stdio`), not a change to how the protocol itself is spoken.

**The DAP side needed a real fix, not just a new test.** The first attempt at `same_dap_client_hits_a_real_breakpoint_in_a_real_python_program` hung and timed out, reproducibly, every time. Diagnosed rather than retried-until-passing: `launch_and_break` sent `launch` and synchronously blocked waiting for its response before sending `setBreakpoints`/`configurationDone` — which is exactly correct for `lldb-dap` (responds to `launch` immediately) but deadlocks against `debugpy`, which — confirmed by the same manual-probe-before-coding discipline used throughout this document — defers its `launch` response until *after* `configurationDone`, per a spec-legal DAP pattern neither adapter is wrong to choose. The fix: send `launch`, keep its sequence number, continue immediately to `setBreakpoints`/`configurationDone` without waiting, then collect the (possibly already-buffered, possibly just-now-arriving) `launch` response afterward. This is a genuine interoperability finding that would have been missed testing against only one adapter — the entire reason §39.2 chose to flag registry replication as an open risk rather than assume the pattern generalizes for free. After the fix: all of `dap-spike`'s and `lsp-spike`'s tests (Rust and Python both) pass, repeatedly, run per-crate.

**What this confirms**: the in-house LSP/DAP client architecture (§2.3) is not merely correct for one language by coincidence of matching one adapter's assumptions — it generalizes to a second, differently-behaved pair of tools, and the one place it didn't generalize for free was found and fixed by testing against a real second adapter rather than assumed away. This directly de-risks §40.2's "Replicate LSP client x5 / Replicate DAP client x5" backlog items: the pattern replicates, provided each new adapter actually gets protocol-probed first rather than assumed to match the first one's behavior — which is now a documented lesson, not just a hoped-for one.

**What this still does not confirm**: any language beyond Rust and Python, any adapter-ordering quirks beyond the one found (conditional breakpoints, multi-threaded stepping, and incremental sync are still untested on either language), and — unchanged — spike 0.1's GPU half and spike 0.4, both still blocked on the same missing display/GPU.

### 47.8 A Reproducible Hover Flake, Root-Caused and Fixed (a later-session audit, not a new spike)

A routine `cargo test --workspace --release` run during a later, unrelated audit pass (renaming the project and sweeping the whole repo for real bugs, not a spike-development session) failed once, in `lsp-spike`'s `hover_returns_real_type_and_signature_info`. §47.6 had already logged a superficially similar single observed flake in the whole-workspace run and left it "not fully root-caused." This time, rather than accept that verdict again, the single test was re-run in isolation ten more times: it failed roughly one run in four to five, definitively ruling out §47.6's "cross-crate CPU contention only" theory, since these runs used no other workspace member at all.

**Root cause, actually diagnosed**: `wait_real_diagnostics` proves rust-analyzer's diagnostics pass has converged for the file, but hover at a specific call site is answered by a separate, on-demand type-inference/resolution query that is not guaranteed to have converged at that same instant — the two are different internal analysis passes triggered by different requests, and nothing in the LSP spec (or rust-analyzer's behavior) promises the second is warm just because the first finished. The test's assumption — "diagnostics-ready implies hover-ready" — was simply false under load, not flaky in the sense of being inherently unreliable.

**The fix**: not a longer timeout or a retried whole test, but the same *poll-for-real-readiness* pattern `wait_real_diagnostics` itself already uses, applied at the one point that actually needed it — the test polls `hover` up to 20 times at 250ms intervals (bounded 5s ceiling) until real markdown contents come back, rather than trusting the first response. Verified, not assumed: 16 consecutive isolated runs after the fix, zero failures (previously ~1-in-4).

**What this confirms**: the earlier "not fully root-caused" flake in §47.6 had at least one real, now-fixed contributor — a genuine race condition, not merely sandbox resource contention as originally guessed. Whether cross-crate contention under full-workspace parallelism is *also* a contributing factor remains unconfirmed either way; this subsection fixes the one race that was actually reproduced and diagnosed, not the whole hypothesis space §47.6 raised.

### 47.9 Spike 0.1 (GPU half) — First Increment Actually Executed, in a Much Later Session

§47.1 closed with "the GPU half... requires a real display and GPU that this sandbox doesn't have. This spike is not closed." Every session since repeated that as settled fact. A much later session checked again rather than trusting the carry-forward claim — same discipline §47.5 already modeled for the DAP half of Spike 0.2 — and found a real GPU reachable after all: a standalone `wgpu 0.19` adapter-enumeration-and-device-creation probe succeeded against `Intel(R) UHD Graphics 620` over Vulkan, `IntegratedGpu`, not a software/CPU fallback. That result is itself worth stating plainly: an assumption this document carried as settled across many sessions was specific to the sandboxes that wrote it, not a fact about the project.

**What was built**: `spikes/render-spike`, a real Cargo crate — a `wgpu`/`winit` window rendering the real, already-tested `spartan_buffer::Document`'s content via `glyphon` (real text shaping and GPU rasterization, not a placeholder), real keyboard-driven edits, and a real second render pipeline drawing a caret positioned every frame from the cursor's actual char index via `cosmic-text`'s own layout data. Explicitly scoped down from §39.1's full 3-4 week/2-engineer spec to a first, honest increment — full details, all real numbers, and an explicit "what this does not confirm" section live in `spikes/render-spike/README.md` rather than duplicated here.

**Two real bugs found by running it, not by inspection**: an sRGB gamma mismatch (wgpu's clear-color and this spike's cursor-fragment color are linear-space values passed against an sRGB surface, which gamma-encodes on write — an intended dark background rendered visibly lighter until corrected with a real sRGB EOTF conversion); and a genuine semantic mismatch between `ropey` (this project's `Document` foundation, §2.1) and `cosmic-text` over how many lines a file ending in `"\n"` has — ropey counts one more, empty phantom line after the final newline than cosmic-text ever lays out, so a naive cursor-position lookup silently drew no caret at true end-of-file on such documents until the phantom-line case was detected and handled explicitly. Neither is a bug in either library; both are real seams between independently-correct tools, found only because a cursor was actually driven to end-of-file and screenshotted, not reasoned about.

**A real, measured result against §39.1's own success criteria — and an honest miss, not a pass**: run against the identical 50,000-line synthetic corpus `rope-spike` uses (`fixture::synthetic_file`, ported verbatim, byte-for-byte the same generator), a 2000-sample internally-scripted latency benchmark measured p50=169.3ms / p95=195.8ms / p99=223.9ms / max=308.7ms input-to-photon, with a 897.7ms cold-open (process start to first presented frame) — both far outside §39.1's stated targets of p99 <5ms and cold-open <100ms. This is not a subtle miss or a measurement artifact: an independent 500-sample cross-check run immediately after landed in the same ballpark (p50=185.1ms, cold-open=899.7ms) without matching to the decimal, which is itself evidence the numbers are real measurements rather than fabricated or reused ones. The cause is a named, deliberate first-increment shortcut, not a mystery: this spike re-shapes the *entire* document on every edit, with no damage-region tracking at all, and latency scales visibly with document size across the session's runs (34 bytes: ~1.2ms p50; 20.5KB: ~9.2ms p50; 3.5MB: ~169-185ms p50) — exactly the signature of that shortcut, and exactly the kind of honest negative result a spike exists to surface before the real damage-region renderer is built.

**What this confirms**: the GPU half of Spike 0.1 is no longer "never run" — it has real, repeated execution on real hardware now, using the same `Document` API the rest of the project already depends on, producing a real (if currently target-missing) latency baseline. The instrumentation itself is trustworthy — numbers are non-zero, vary realistically between independent runs, and respond predictably to a real load variable (document size) — which matters as much as the numbers themselves: a benchmark harness that can't be trusted to measure real cost is worse than no harness.

**What this still does not confirm**: §39.1's <5ms p99 / <100ms cold-open targets, which this increment misses by roughly 45x and 9x respectively at 50k lines — closing that gap needs real damage-region re-rasterization, not implemented here; a literal per-glyph SDF atlas (`glyphon`'s coverage-mask atlas is a related but different technique, named explicitly rather than conflated); the full keystroke-trace corpus (held-key repeat, large-block paste, rapid undo/redo, scroll-while-typing — only single-character random-position inserts were run at benchmark scale); rope memory overhead vs. a flat buffer (not measured this pass); more than one machine or one integrated GPU; and a formal go/no-go recommendation, which isn't warranted given the gaps above. **Spike 0.1 is still not closed. This is a second, honestly-scoped slice of it — the GPU half's first real numbers — not its completion.**

### 47.10 Spike 0.1 (GPU half) — Damage-Region Increment, Same Session

§47.9 named its own biggest gap plainly: full-document reshape on every edit, missing the <5ms p99 target by ~45x at 50k lines. Rather than leave that as a permanent shrug, the same session researched whether cosmic-text's own public API supports anything cheaper before reaching for a fork. It does, partially: `cosmic_text::BufferLine::set_text()` invalidates only that one line's cached shape/layout, and `Buffer.lines: Vec<BufferLine>` is public, so a single-line edit can bypass `Buffer::set_text()`'s full-document rebuild entirely. What the same research also found, and confirmed by reading `glyphon::TextRenderer::prepare()`'s source rather than assuming: it unconditionally walks every *visible* line's `layout_runs()` and re-uploads its glyphs on every call, with no scoped/partial API — so the research's own prediction going in was that GPU-upload cost would remain dominant even after fixing CPU-side shaping, and a real <5ms result should not be expected from this alone.

**What was built**: `EditorView`'s edit methods now return an `EditEffect` (`Line(usize)` / `Structural` / `None`) classifying whether an edit changed the document's line count. Same-line edits route through a new `TextState::set_line_text`, which calls `BufferLine::set_text` directly on the one changed line; structural edits (a newline inserted or removed — cosmic-text has no public API for cheap line insert/delete) still fall back to the original full `set_text()`.

**A third real bug, found by running it, not by inspection, in the same family as §47.9's second one**: immediately after Enter creates a new line, `Document` (ropey) reports the cursor as being on that new line right away — but cosmic-text's `buffer.lines` isn't extended to include it until the next full rebuild processes that content. Calling the new per-line update with that not-yet-existing line index silently no-op'd (`Vec::get_mut` returning `None`), which silently dropped the very next character typed after pressing Enter. Found by literally pressing Enter, typing `abc`, and watching it fail to render — not predicted in advance despite already knowing about the closely-related ropey/cosmic-text line-count mismatch from §47.9, which is itself a reminder that knowing a class of bug exists in one code path doesn't automatically surface every place it recurs. Fixed with an explicit bounds check (`TextState::line_count()`) before using the fast path, falling back to a full reshape otherwise; locked in with a headless regression test that doesn't need a GPU to run.

**The real, measured result contradicted the research's own prediction, in the good direction**: re-run against the identical 50,000-line corpus and scripted-benchmark methodology as §47.9, p50 dropped from 169.3ms to ~3.0ms (~56x), p95 from ~195.8ms to ~5.6ms (~35x), and p99 from 223.9ms to ~12.2ms (~18x) — a much larger improvement than "fixing CPU shaping alone, with GPU upload still dominant" would predict. The real explanation, once measured rather than assumed: at 50,000 lines, `Buffer::set_text`'s full re-parse-and-reshape of the *entire* document on every keystroke was apparently the larger share of the original cost, not the GPU upload of the ~35 actually-visible lines. An independent 500-sample cross-check run landed in the same ballpark (p50=2.19ms, p99=5.99ms) without matching to the decimal, the same anti-fabrication check §47.9 already used. Cold-open (~900-1300ms vs. a <100ms target) is unchanged, since it was never on the per-edit code path this pass touched.

**What this confirms**: real per-line damage-region CPU shaping is achievable against cosmic-text's existing public API with no fork required, and closes most of the p99 gap this spike's own prior report identified as its biggest shortfall. It's also a caution worth stating plainly for future work in this codebase: a documented, real limitation in a dependency (glyphon's un-scoped GPU upload) does not by itself tell you how much of the total cost it accounts for — that still has to be measured, not inferred from which limitations are known.

**What this still does not confirm**: §39.1's <5ms p99 target, which this increment approaches but still misses (p99 measured at 6-12ms across two runs, down from 224-266ms); cold-open, completely untouched by this pass; the GPU-upload cost itself, which remains real and would need patching `glyphon` (not attempted, a materially bigger and riskier undertaking than reusing cosmic-text's own public API) to address further; and everything else §47.9 already listed as unconfirmed (keystroke-trace corpus, per-glyph SDF, rope memory overhead, multi-machine coverage, a formal go/no-go). **Spike 0.1 remains open. This closes more of the GPU-half's gap than §47.9 did, but is still not the spike's completion.**

### 47.11 Spike 0.4 — First Increment Actually Executed, Same Session

§47.3 recorded Spike 0.4 as "not executable in this sandbox" — needing a display server and GPU-capable windowing environment neither available at the time. That blocker is the same one §47.9 already found stale for Spike 0.1's GPU half, on the same later-session machine with a real reachable GPU and display. Rather than assume Spike 0.4 was still blocked by the same old note, the same session attempted it for real.

**What was built**: `spikes/ui-shell-spike`, a real Cargo crate — a native `wgpu`-rendered two-panel shell (left rail, auxiliary pane) with a real embedded `wry::WebView` (real WebView2, not a placeholder) occupying the center-stage area, real bidirectional state sync between native Rust and the WebView's JavaScript (a native keypress pushes a value into the page via `evaluate_script`; the page's button posts back via IPC, which Rust receives and acknowledges), and a real mode-toggle color cross-fade timed against §8.4's stated ~180ms duration. Full detail, real numbers, and an explicit "what this does not confirm" section live in `spikes/ui-shell-spike/README.md` rather than duplicated here.

**Two real, previously-unknown integration gaps found by running this, not by inspection**: (1) `wry`/`webview2-com` compiles cleanly on this project's Windows GNU toolchain, but the resulting executable fails at runtime with `STATUS_DLL_NOT_FOUND` — root-caused via `objdump -p` import-table diffing (not guessed) to a missing `WebView2Loader.dll`, a real Microsoft loader stub that `webview2-com-sys`'s own build script already stages into its own build output but which Cargo has no mechanism to place next to the final executable automatically; fixed with a `build.rs` that replicates that copy step, verified working on a clean build. (2) Once the embedded WebView2 control takes keyboard focus, clicking the native (non-WebView) part of the *same* top-level window does not return keyboard focus to the native event loop — confirmed with an isolated test crate that the window remains the OS foreground window throughout, yet `KeyboardInput` events stop arriving entirely; `winit`'s own `focus_window()` does not fix this (tested), a direct Win32 `SetFocus` call on the window's HWND does (tested, confirmed).

**The second finding is not a minor bug — it is a concrete, first real instance of the exact risk §35.9/§39.4 name as Spike 0.4's central uncertainty**: "does this feel like one app," here surfacing as keyboard input ownership silently getting stuck on the WebView rather than a visual seam. That it has a small, working fix is a genuinely useful result; that the fix was found by accident of testing rather than designed in from a specified focus-ownership contract is the honest caveat carried into the README.

**A real, measured result against §39.4's own success criteria**: WebView state round-trip (measured end-to-end within JavaScript's own clock, avoiding cross-clock skew against Rust's `Instant`) came in at p50=2.3ms, p99=3.5ms, max=8.4ms across 10 real samples — comfortably under the <50ms target. Mode-switch fade duration measured 180.4-180.7ms against a requested 180ms, under the <200ms perceived-switch-time target. Both are real, instrumented, reproducible numbers, not estimates.

**What this confirms**: a native `wgpu` shell and a real embedded WebView2 control can coexist in one window with bidirectional state sync well within the spec's latency target, once the two integration gaps above are addressed — both now fixed and documented, not just found.

**What this still does not confirm**: the real three-column skeleton (only two solid-color rects stand in for chrome), the real mode-switch treatment (§8.4's cross-fade-plus-scale, applied here only as a color interpolation on a native panel, not the real center-stage content), the real `CanvasEdit` state model (§6.1-6.2 — this spike's counter proves the bridge mechanism, not the real event shape), a *designed* (as opposed to reverse-engineered) focus-ownership contract for native↔WebView handoff, and the written qualitative "does this feel like one app" verdict §39.4 names as its actual exit artifact — the honest partial answer this pass can offer is that the *mechanism* feels solid while the *visual* integration does not yet resemble one coherent app, since building that resemblance was out of scope here. **Spike 0.4 is not closed. This is its first real, executed slice.**

### 47.12 Spike 0.3 — Real Local-Model Fidelity, First Data, Same Session

§47.2 named the single biggest gap in the fallback-parser work plainly: "the actual ≥80% real-world tool-call fidelity target against a real 7B/13B local model... requires an actual Ollama instance," unavailable in every session up to that point. The user asked to retry Ollama; it turned out to already be genuinely installed and running (`ollama.exe`, version 0.31.1, real HTTP service on `localhost:11434`) — this session pulled a real model and drove the existing, already-tested `FallbackParser` (§47.2) against its real output for the first time.

**Scope named honestly up front**: disk space on this machine was down to ~11-12GB free at the time (see CLAUDE.md's Spike 0.3 status note), nowhere near enough to safely pull a 13B-class model, and tight even for a 7B-class one alongside everything else already on this disk. The model actually used, `llama3.2:1b` (1.2B parameters, Q8_0, 1.3GB), is a real but *smaller* class than §39.3's literal "~7B class, ~13B class" targets — stated plainly in the new test's own doc comment, not silently substituted.

**What was built**: `spikes/fallback-parser-spike/tests/real_ollama_fidelity.rs`, a real integration test that self-skips (matching `dap-spike`'s/`lsp-spike`'s established pattern) if Ollama isn't reachable or the model isn't pulled, otherwise sends five real prompts to a real local model via real blocking HTTP calls (`ureq`, no mocked server) and feeds each real response through the actual `FallbackParser` — the same component §47.2 could previously only test against synthetic, hand-written token streams.

**Real, honest, and not flattering results**: of three prompts that should produce a tool call, only 2/3 produced syntactically valid JSON the parser could extract a tool name from at all — the first attempt emitted Python-style call syntax (`read_file(path="src/main.rs")`) instead of the required JSON, which the parser correctly caught and surfaced as `ToolCallFailed` with reason "invalid JSON," exactly per §3.4's non-negotiable "never silently drop" requirement, not a parser bug. Of those, **0/3 chose the semantically correct tool**: one used `"run-terminal"` (hyphen) instead of the real tool name `"run_terminal"` (underscore) — valid JSON, wrong string, a real hallucinated-tool-name failure the parser has no way to catch since validating tool names against a whitelist isn't its job; one was asked to *create* a new file and called `read_file` instead of `edit_file` — a real, repeatable failure mode (a separate manual trial against the same model produced the identical wrong-tool choice for the same kind of task, which is why it's now a permanent case in the automated suite rather than a one-off anecdote). A control prompt ("what's 2+2") correctly produced plain text with no spurious tool call.

**What this confirms**: the fallback parser's synthetic-token-stream test suite (§47.2) generalizes to real model output for the cases it was designed around — real invalid JSON gets surfaced, not dropped; real valid-but-wrong-tool-name JSON parses correctly (extracting exactly what the model actually said, which is the parser's job, not tool-name validation). The parser itself has no bugs surfaced by this real test.

**What this does not confirm**: §39.3's actual ≥80% fidelity target, tested here against a materially smaller (1.2B, not 7B/13B) model with a 0/3 correct-tool-choice result on a 3-prompt sample — far too small a sample and far too small a model to be read as a verdict on real 7B/13B-class fidelity one way or the other. No comparison against the `ClaudeProvider` path (§39.3's other half). No `ModelProvider` trait, no `OllamaProvider`, no actual agent loop — this test drives Ollama's raw HTTP API directly, not through any of Spartan's own abstractions, none of which exist yet. **Spike 0.3 remains open.** This is a first, small, honestly-scoped real data point — largely a negative result at this model size — not a fidelity report on the models the spec actually names.

---

## 48. Debugging Log — A Real Bug, Found and Fixed

Rather than another feature sweep, this pass went back into last turn's actual code and tried to break it. It worked.

### 48.1 The Bug

The fallback parser's tail-keep logic (§3.4, exercised in Spike 0.3) buffers a small window of bytes while scanning for a fence marker split across streamed chunks. The cut point was a raw byte offset — `scan_buf.len() - keep` — sliced directly into the string. Leo's own prose routinely contains non-ASCII characters (em-dashes, emoji, non-English text), and Rust panics if you slice a `&str` at a byte index that isn't a valid UTF-8 character boundary.

First attempt to reproduce it with a generic sweep across padding lengths found nothing — a genuine near-miss, because the test was structurally incapable of ever landing the cut point inside the multi-byte character (the trailing ASCII content was long enough to always keep the emoji outside the tail window). Recomputing the exact byte arithmetic and targeting suffix lengths 16–18 specifically reproduced it immediately:

```
thread '...' panicked at src/lib.rs:65:76:
byte index 4 is not a char boundary; it is inside '🐛' (bytes 1..5) of `x🐛bbbbbbbbbbbbbbbbbb`
```

### 48.2 The Fix

Snap the split index down to the nearest valid char boundary before slicing, rather than trusting the raw arithmetic:

```rust
let mut split = self.scan_buf.len() - keep;
while split > 0 && !self.scan_buf.is_char_boundary(split) {
    split -= 1;
}
```

This only ever retains a few extra bytes in the buffer one iteration longer — it never drops or misreads data. Two regression tests now cover it permanently: one sweeping every suffix length that could reproduce the exact panic, one confirming byte-for-byte that no content is ever lost across the tail-keep boundary, only deferred to the next flush. Full suite: 10/10 passing, including both.

### 48.3 What This Actually Demonstrates

The first sweep test looked reasonable and passed cleanly — and proved nothing, because it didn't actually exercise the failure condition. That's worth sitting with: a passing test suite is only as good as whether the tests were actually capable of finding the bug, which is a real and easy way to end up with false confidence in exactly the kind of transparency-focused system this whole document keeps insisting on.

### 48.4 New Features This Directly Motivates

- **Fuzzing / property-based testing as a first-class Test Studio capability** (amends §24): a fuzz corpus panel wrapping `cargo-fuzz`/`libFuzzer` for Rust components and equivalent property-testing frameworks per language, seeded specifically with adversarial Unicode/multilingual/emoji-heavy text for anything in the text-processing path (rope, this parser, the LSP transport layer) — a fuzzer finds this exact class of bug in milliseconds; it took a manual reasoning pass and one failed attempt to find it by hand. Given Spartan's own architecture leans hard on custom string-handling code (the rope, this parser, the LSP JSON-RPC framing), this isn't a generic nice-to-have — it's aimed at the parts of the codebase most likely to have exactly this kind of bug
- **A Known Limitations ledger** (amends §36.5's release-gate checklist): a first-class, versioned artifact type listing discovered-but-not-yet-fixed edge cases — for instance, this parser's fence-detection still naively terminates on the first ` ``` ` inside a tool call's JSON body, which would misfire if a string argument itself contained a triple-backtick sequence. That's a known, real, minor limitation, and per this document's own stated philosophy it belongs in a visible ledger, not silently left for someone to eventually rediscover the hard way

---

## 49. Signature Differentiators

Three ideas selected out of the seven proposed last turn, on the stated bet: one already has a technical proof behind it, one is nearly free because the engineering is already done, and one is the cheapest possible win since the architecture already supports it. Built as a real interactive artifact (`spartan-signature-features.jsx`) alongside this spec, not just described.

### 49.1 Timeline Scrubber (amends §4.2, §43.1)

Drag between implementation attempts and watch the file switch live, with real measured stats (latency, test pass count) per attempt — not a static side-by-side diff. This is directly underwritten by a number this document actually earned rather than assumed: Spike 0.1 measured `rope.clone()` at **p99 = 0.0002ms** (§47.1), confirming structural sharing is cheap enough that scrubbing through history can feel instant rather than triggering a visible reload. Feeds from the same checkpoint mechanism as Experiment Mode (§43.1) and the Automated Implementation Bake-off — this is that feature's UI made tactile.

### 49.2 Trust Card (amends §36, §8.6)

A first-launch page stating exactly what's guaranteed and why, sourced directly from §36's hardening — Model Integrity, Single Writer Invariant, Path-Jailing, Untrusted-Repo Quarantine, Transparent Pricing, No Forced Updates — each tagged as *"verified by release gate §36.5,"* not asserted as marketing copy. Replaces the generic onboarding modal referenced in §8.6 for the specific moment a new user is deciding whether to trust an agentic tool with their codebase, which research into competitor failures (§36.2) suggests is exactly the moment that matters most and is currently unaddressed by anyone in this category.

### 49.3 One Leo, Felt (amends §46.2)

§46 made the architectural claim that CLI and desktop sessions share one store. This makes that claim visible at the exact moment it would otherwise go unnoticed: opening the desktop app surfaces *"Picking up from your CLI session · 14 min ago · 3 files changed · tests passing"* rather than silently reflecting shared state with no acknowledgment. The distinction matters — competitors' CLI and desktop tools are typically architecturally separate products sharing a brand name; this is one architectural claim, made felt rather than left to be inferred from behavior.

---

## 50. Antigravity 2.0 Fidelity Pass — Corrected & Sharpened

Re-verified against current sources rather than relying on the earlier research pass. The core structural claims already in §8/§22/§36.2 hold, but with more precision than originally captured — worth tightening rather than leaving approximate.

### 50.1 What Was More Precise Than First Captured

- **The left column is two things, not one.** Antigravity 2.0's actual Agent Manager splits it into **Workspaces + a Playground** (top) and an email-inbox-styled **task list** (middle) — each agent task genuinely modeled as its own thread, the way an inbox treats messages. The prototype's left rail (§8) only had a flat session list; it's been amended to match this two-part structure — a `WORKSPACES` strip (with an explicit, low-stakes **Playground** entry) sitting above the renamed **Inbox** section, in `spartan-interface-prototype.jsx`.
- **The Editor/Agent-Manager split is a real keyboard-level toggle**, `Cmd+E`, between what are — critically — now two genuinely separate top-level products in the 2.0 lineup: the standalone **Antigravity** app (agent-only, no IDE) and **Antigravity IDE** (the original VS Code-based combined app). This confirms, more precisely than before, that the "forced split into two apps" concern in §36.2 isn't a mischaracterization — 2.0 deliberately productized the split rather than accidentally causing it.
- **Fast mode vs. Planning mode** is a real, explicit per-task toggle before submission, not just an internal implementation detail — worth reflecting as an explicit control in Spartan's own task-submission UI alongside the existing autonomy-level settings (§42.2's Agent Behavior category), not just inferred from a global setting.

### 50.2 One New Hardening Item, From a Real Confirmed Issue

Independent verification surfaced a documented Antigravity vulnerability class not previously in §36's catalog: agent-rendered external content (e.g., an image reference embedded in repo content or AI-generated output) can be used to exfiltrate data via a crafted URL, compounded by the agent disregarding `.gitignore` scope and, under auto-approval, fetching the external resource without a pause — reportedly first characterized by Google as "intended behavior" before reconsideration. This is a distinct attack surface from anything in §36.4's existing hardening (path-jailing covers filesystem traversal; nothing yet covers outbound fetches of *content the agent didn't write but is rendering*).

**New mechanism — External Content Fetch Gating** (amends §9, §36.4.6): rendering any externally-resolvable reference (image URLs, remote embeds) encountered in repo content, model output, or artifacts requires the same tool-approval posture as a network-capable tool call — never auto-fetched merely because it's being *displayed* rather than *executed*. `.gitignore`-scoped content is excluded from agent context by default, not just from version control, unless explicitly widened by the user.

### 50.3 High-Contrast, Antigravity-Simplified Theme (a later-session request, researched rather than guessed)

A later request asked for Spartan's default theme to be "high contrast" and aesthetically "identical to" Antigravity 2.0, in service of simplifying and decluttering the interface. Rather than guess at Antigravity's actual token values from training-data recall, this was researched fresh: Antigravity's documented design-system values are background `#09090B`, surface `#18181B`, and border `#27272A` — a minimal, high-contrast, neutral-gray palette with generous whitespace, plus a project-level `DESIGN.md` the agent reads automatically (a pattern Spartan already independently arrived at via Open Design integration, §6/§38 — confirmed convergent, not copied).

**Adopted, near-exactly**: `interface-prototype.jsx`'s `bg`/`s2`/`border` tokens are now `#09090B` / `#18181B` / `#27272A` — matching the researched values rather than approximating them. `textDim`'s luminance was raised from the previous pass for the same reason: "high contrast" is a claim about the whole token set, not just the background.

**One deliberate divergence, named rather than silently mismatched**: Antigravity's own accent is reported as purple; Spartan keeps its own established accent (a rust/terracotta hue used consistently across every fleet-engine tag, status color, and badge already built into the reference prototype across many prior passes). Repainting that would be a full rebrand of an already-coherent identity, not a contrast improvement — the request's operative goal ("simplify and unclutter") is served by the neutral-palette and whitespace changes, not by the specific accent hue. This is easy to override if the literal purple match is actually wanted; flagged here rather than silently decided.

**Declutter moves actually made, not just claimed**: `LayerTag` (the `Global`/`Project`/`Session`/`System` pill shown on every settings row — 47 call sites) dropped its boxed background/border in favor of plain color-coded text; the same information survives, with far less simultaneous visual chrome across a long settings list.

**What this explicitly does not mean**: matching Antigravity's minimalist token language is not the same as matching a specific, real usability regression a later Antigravity 2.0 pass reportedly shipped — see the new failure-catalog row in §36.2 and the permanent boundary named in §36.4.10. Sources: Google Developers Blog, TechCrunch, and Antigravity's own settings/design docs for the feature and token claims; DeepakNess and a developer-backlash writeup for the minimalism-regression claim — cited rather than asserted from memory, consistent with this document's own discipline against fabricating specifics it hasn't checked.

---

## 51. Full System Audit

Ran a genuine structural audit rather than asserting completeness — the same discipline as §48's debugging session, applied to the whole document and the reference interface.

### 51.1 What the Audit Actually Found (Spec)

A real bug: **sections 44, 45, 46, and 49 each existed twice**, with different wording per copy — the LiteLLM/Vibe/CLI content from one turn and a near-duplicate from another had both survived, non-sequentially numbered (44/45/46/44/45/46/47/48/49/49/50, 54 headers total instead of 50). This is exactly the "silently diverging sources of truth" failure mode §36 argues against, just inside this document instead of inside Spartan itself.

**Fix, not just deletion**: compared both copies, kept whichever one later sections actually cross-referenced correctly (§49.3 cites §46.2's "Two Operating Modes," which only existed in one copy), merged the genuinely unique valuable content from the other copy in as new subsections (§44.6 Virtual Keys, §45.5 "Formalize This Session," §46.6 Auth on Headless Machines) rather than discarding it, then removed the stale duplicates.

**A second bug, found immediately after fixing the first**: the merge itself dropped two section headings (`## 45.` and `function SettingsView`) where a `str_replace` boundary swallowed a line it shouldn't have — caught both by re-running the same structural check rather than assuming the fix worked, exactly the "don't trust a pass without verifying it could have failed" lesson from §48.3.

**Verified clean**: 50 sections, sequential 1–50, zero duplicates, exactly one `End of spec` marker, no cross-reference pointing to a section number beyond 50.

### 51.2 What the Audit Found and Fixed (Interface)

| Gap | Status before this pass | Status now |
|---|---|---|
| Settings categories | 3 of 16 had real content, 13 were placeholder text | All 16 have genuine interactive controls |
| Vibe Mode | Spec-only | Real 4th autonomy button in Agent Behavior settings |
| LiteLLM | Spec-only | Listed as a connected provider in Leo & Models settings |
| External Content Fetch Gating (§50.2) | Spec-only | Real toggle in Privacy & Security settings |
| Workspace rail (Test/Ops/Data/Manage) | Not in the mode toggle at all | Reachable via a "More" dropdown, each a labeled stub |
| Debugger | Not represented | Clickable breakpoint gutter, Run button, real call stack + variables panel on a simulated hit (§32) |
| ADB Device Panel | Not represented | Devices/Logcat/Shell tabs, explicitly labeled "3 of 8 tabs shown" rather than implying full §33 coverage |
| Project Graph (§30) | Abstract only | A link strip on the active file showing its linked ticket, test count, and deploy recency |

### 51.3 What's Honestly Still Reference-Only

Stating this plainly rather than letting the above list imply totality: full depth on all 8 ADB tabs, the complete debugger feature set (conditional breakpoints, watch expressions, time-travel), Test/Ops/Data/Manage beyond their current labeled stubs, the WASM plugin marketplace, and the Enterprise settings category (omitted from the interactive category list entirely, since it's spec'd as visible only when enterprise mode is active and there's no tenant state to activate it against in a static mockup) are not built to interactive depth anywhere in this repository. That's not an oversight being glossed over — it's the same progressive-disclosure principle real IDEs use, stated explicitly instead of left ambiguous. A single reference mockup demonstrating the full architecture's shape and a production IDE with complete depth on all fifty-one sections' worth of features are different deliverables; this audit closes the gap on the former and is honest about not being the latter.

### 51.4 Confidence Statement

What can be said with actual verification behind it, not just confidence: the spec's structure is internally consistent (checked, not assumed), the reference prototype compiles cleanly after every edit in this pass (checked via Babel after each change, twice catching a real self-introduced bug before it shipped), and every settings category, workspace view, and panel referenced in this turn's summary actually exists and is clickable in the delivered file. What cannot honestly be claimed: that every one of fifty sections has matching interactive UI, or that no other inconsistency exists anywhere in a document this size that wasn't specifically checked for. The audit covered numbering integrity, cross-reference validity, and the specific gaps named last turn — it was not an exhaustive line-by-line re-read of all fifty sections against the interface, which would be a materially larger undertaking than this pass.

---

## 52. External Agent Fleet — Third-Party CLI Orchestration (amends §3, §30, §42, §46)

Before this pass, the repository this spec now lives in *was* a different, shipped product: "Spartan IDE Agent Deck Console," an Electron shell around a terminal launcher (`agent-deck`) that runs third-party AI CLIs — Claude Code, Codex, Gemini CLI, OpenCode, Aider, Copilot CLI, Cursor Agent, Qwen Code, Amp, Goose, Crush, Continue, OpenHands — as managed sessions, with per-engine usage tracking, automatic failover, and a web cockpit. That product is being replaced by the from-scratch architecture in §1–§51, per this session's mandate. It is not being discarded: its verified, real code is preserved at `legacy/agent-deck-console/` (§55), and its capability is folded into this architecture as a first-class subsystem rather than left behind as a feature regression.

### 52.1 Why This Is Not Just Another ModelProvider

§3's `ModelProvider` trait models API-level backends Leo talks to directly (Claude, Ollama, anything LiteLLM fronts per §44) — Spartan controls the request/response loop token-by-token. A third-party CLI like `codex` or `aider` is a different shape entirely: a full interactive terminal program with its own UI, its own approval prompts, its own model choices, running as a subprocess Spartan supervises but does not converse with over an API. Modeling it as a `ModelProvider` would be dishonest — Spartan can't intercept its tool calls or apply §4.5's tool-execution sandbox to actions the external CLI takes internally. It is therefore a distinct concept: a **Fleet Engine**, supervised at the process/session level, not the token level.

### 52.2 Fleet Session Model

```rust
struct FleetEngineProfile {
    id: EngineId,                 // "codex", "gemini-cli", "aider", ...
    display_name: String,
    detect: CommandSpec,          // probe used to confirm the binary is on $PATH
    launch: CommandSpec,          // the actual invocation, args templated per-session
    color_tag: String,
    fallback_chain: Vec<EngineId>,// engines to try next on quota/rate-limit failure
}

struct FleetSession {
    id: SessionId,                // same SessionId space as native Leo sessions
    engine: EngineId,
    pty_handle: PtyHandle,        // supervised subprocess, not a raw fork
    cwd: PathBuf,                 // path-jailed per §4.5, no silent `cd ..` escape
    usage: UsageCounters,         // tokens/requests where the CLI's own output exposes them
}
```

- Fleet sessions appear in the **same left-rail session list** as native Leo sessions (§8, §30's Project Graph), tagged with the engine's color and name instead of the Leo/Claude badge — one unified place to see "everything working on this project right now," whether it's Leo or a supervised third-party CLI.
- A Fleet session is a **PTY-attached subprocess**, not a headless API call: the user can attach to its live terminal output the same way `agent-deck session attach` worked, or run it detached and check back later.
- **Registry file**: `.spartan/fleet-clis.toml` (project-level) and `~/.spartan/fleet-clis.toml` (global), successor to `config/ai-clis.tsv` (§55) — same "one row per tool, tab-separated becomes one table per tool, TOML" idea, now versionable per-project and mergeable with the global list rather than a single flat file.

### 52.3 Usage Tracking & Auto-Switcher (amends §18)

- Per-engine token/request counters, parsed from each CLI's own status output where available (best-effort, since Spartan doesn't control the CLI's internals) and surfaced in the same Cost/Usage Dashboard (§18) as native Leo sessions, with a clear "external, self-reported" badge distinguishing it from Spartan's own precisely-metered Claude/Ollama usage.
- **Auto-switcher**: when a Fleet engine's launch or mid-session output matches a configured quota/rate-limit signature (regex per engine profile, since error formats aren't standardized across CLIs), Spartan offers — never silently performs — a one-click switch to the next engine in `fallback_chain`, starting a new Fleet session with the same working directory and task description carried over. This mirrors the old Agent Deck auto-switcher's intent but keeps the human in the loop on the actual cutover, consistent with §9's "never silent" principle for anything provider-routing-related.

### 52.4 Security Posture

Fleet subprocesses run under the exact same sandbox model as §4.5's `run_terminal`: working directory locked to project root, no automatic secret/`.env` injection, timeout and output-size caps. Because Spartan cannot see or gate a third-party CLI's *internal* tool-call approvals, Fleet sessions are treated as **less trusted by default** than native Leo sessions — no Fleet engine is eligible for the fully-autonomous approval tier (§4.1's `AwaitingApproval` matrix) regardless of user settings; destructive-operation confirmation always surfaces at the Spartan process-supervision layer (e.g., confirming before Spartan itself would let a Fleet session's working directory include `.env`/secrets paths), independent of whatever the external CLI's own prompts claim to have confirmed internally.

### 52.5 Roadmap Placement

This is **Tier 2** work per §35's prioritization discipline — it depends on the native session rail, Project Graph, and settings infrastructure from Tier 1 existing first. It is not blocking Tier 0/1 and should not be pulled forward ahead of the core editor/agent MVP; §55's parity matrix tracks it explicitly so the capability isn't quietly lost in the meantime.

---

## 53. Neural Link — Workspace Analysis Bridge (amends §4.3, §9)

The legacy console's Neural Link (`scripts/neural-link.py`, §55) is a local, explicitly non-autonomous static-analysis bridge: it reads a workspace (this project or another local path the user points it at) and produces a report, without ever initiating network, credential, or lateral-movement actions. That constraint is a feature, not a limitation, and carries forward unchanged.

### 53.1 Design

- A local job (`spartan neural-link analyze <path>`, reachable from the CLI per §46 and from a Settings/Ops panel action) walks a target workspace using the same tree-sitter parse + symbol-graph machinery §2.4 already builds for the active project, rather than a bespoke second parser.
- Output is a structured report (`.spartan/neural-link/reports/<timestamp>.json`) plus a human-readable summary — architecture shape, dependency hotspots, dead-code candidates (reusing §13.2's dead-code finder) — never raw file dumps.
- **Feed queue**: a `.spartan/neural-link/queue.jsonl` append-only log (same append-only discipline as §7's session logs) that project memory (§4.3) can selectively summarize from, so cross-project patterns a user has explicitly asked Neural Link to analyze can inform Leo's suggestions *in this project*, without ever pulling the other workspace's raw content into a cloud provider's context — the summarization step is the privacy boundary, and it is mandatory, not optional.
- Explicitly **out of scope, by design, matching the legacy tool's own stated constraints**: starting any autonomous agent loop, initiating network/credential probing, or any lateral-movement behavior. Neural Link only ever reads and summarizes; it is not a second Leo.

### 53.2 Roadmap Placement

Tier 2, same rationale as §52.5 — useful, self-contained, and not on the critical path to a working core editor/agent loop.

---

## 54. Ops Cockpit — Web Dashboard Companion (amends §23, §18)

The legacy console's Node/Express "Dynamic Cockpit" (`scripts/cockpit-server.js` + `web/`, §55) is preserved in intent as a **companion, read-only web view** of the native Ops View (§23), not a second implementation of Spartan itself — the same "one engine, second surface" philosophy §46 already establishes for the CLI applies here to monitoring.

- Serves a localhost-bound dashboard (fleet/session status, cost & usage from §18, Task Runner output from §20.2) for secondary-screen or mobile glancing while the native app does the real work — never a route for taking actions remotely in v1, to avoid quietly reintroducing an unsandboxed second control surface.
- Reuses the native app's session store and event log (§7) as its data source rather than maintaining separate state, so there is exactly one source of truth for "what is Spartan doing right now," matching §36's core lesson about divergent sources of truth.
- Tier 2/3 (monitoring-only companion, after the native Ops View itself exists) — not a Tier 0/1 item.

---

## 55. Legacy Feature Parity Matrix

This session replaced the repository's prior product (an Electron-based Agent Deck console) with the from-scratch architecture defined in §1–§54, per an explicit instruction to do so while not losing what the prior product actually did. The verified, working legacy implementation is preserved unmodified at `legacy/agent-deck-console/` — not on the build path for the new architecture, kept as the reference for behavior parity until each row below is actually implemented natively.

| Legacy feature | Legacy location | New architecture home | Tier |
|---|---|---|---|
| Multi-engine CLI launcher (Claude Code, Codex, Gemini CLI, Aider, Copilot CLI, OpenCode, Qwen Code, Amp, Goose, Crush, Continue, OpenHands, ...) | `legacy/agent-deck-console/bin/spartan-agent-deck`, `config/ai-clis.tsv` | §52 External Agent Fleet | 2 |
| Usage tracker & auto-switcher | `legacy/agent-deck-console/scripts/usage-manager.js`, `config/usage-tracker.json` | §52.3 (amends §18) | 2 |
| Dynamic web cockpit | `legacy/agent-deck-console/scripts/cockpit-server.js`, `legacy/agent-deck-console/web/` | §54 Ops Cockpit | 2/3 |
| Neural Link workspace bridge | `legacy/agent-deck-console/scripts/neural-link.py` | §53 Neural Link | 2 |
| AI skills registry (`config/ai-skills.tsv`, `scripts/import-ai-skills.sh`) | `legacy/agent-deck-console/config/`, `legacy/agent-deck-console/scripts/` | Folds into §5's WASM plugin/tool registry and §46's CLI tool surface — a "skill" here is functionally a named tool/command bundle, which §5.3's `agent_api` extension point already generalizes | 2 |
| Cyber-ops visual theme | `legacy/agent-deck-console/web/assets/styles.css` | Superseded by §8's design system (own accent/token scheme); not carried forward as-is, since the new visual identity is a deliberate, separate decision already made in §8 | — |

**Honest status of this section, matching §51's audit discipline**: §52–§54 are design-level amendments written in this pass, reviewed for consistency against §3, §4.5, §9, §18, §30, §35, §42, and §46 as cited inline, but — like the rest of this document per §51.3 — not yet implemented or executed against real third-party CLIs in this environment. Nothing above should be read as "built," only as "specified, and traceable to a real predecessor implementation that already worked."

---

## 56. Git & GitHub Integration — The Source Control Panel (fills a real gap)

"The git panel" is referenced by name four separate times before this section existed — §11's Phase 5 ("DAP + Git panel"), §15's async PR pre-review ("via the git panel's integration"), §23's pipeline status ("in the git panel per branch") — without a single section actually specifying what it is. That's the same "referenced but never defined" failure mode §36 catalogs in other tools, just found here instead of shipped. This section is the actual specification those four references were assuming existed.

### 56.1 Source Control Panel — Local Git

A dedicated dock panel (same shell pattern as the Debug Console and Device Panel, §21.3/§32) rather than a buried menu:

- **Working tree view**: staged/unstaged files grouped separately, each with a status glyph (modified/added/deleted/renamed/conflicted) matching the minimap ticks from §16.1
- **Diff view**: reuses the exact same diff-rendering component as an agent-produced Diff Card (§8.5) — a manual edit and an AI edit look identical when you're reviewing them, on purpose, since they go through the same rope-edit pipeline (§4.5)
- **Stage/unstage/discard** per file or per hunk, commit message box with the project's conventional-commit config (if present) suggesting a type/scope prefix, not enforcing one
- **Branch switcher**: create/checkout/delete/rename, with an uncommitted-changes guard (stash-or-abort prompt) before a destructive checkout — same "never silently discard work" posture as the harness-level git safety rules this project's own tooling already follows
- **Stash**: list/apply/drop, each stash entry showing its originating branch and message
- **Merge conflict resolution**: a three-way view (ours/theirs/result) reusing the Diff Card's accept/reject interaction pattern per conflicting hunk, rather than a separate conflict-resolution UI paradigm to learn

### 56.2 GitHub Layer — Authentication

- Same device-code flow already specified for the headless CLI (§46.6): open a URL, enter a short code, no pasted PAT sitting in a config file or shell history
- Token stored via the same OS-keychain integration used for Hugging Face tokens (§41.4) and signing keys (§21.5) — never in plaintext, never sent to Leo's context window
- Scoped token request (repo + minimal metadata scopes) rather than requesting the broadest available scope by default — a direct application of §36's least-privilege posture

### 56.3 GitHub Layer — Pull Requests

- **PR list** for the current repo, filterable by "mine" / "requesting my review" / "all open" — each row shows title, checks status (pass/fail/pending as a colored strip, not just a word), review state, and linked issue if any (Project Graph, §30)
- **PR detail view**: description, file-by-file diff (same component as §56.1), inline review comments rendered in place, and a **"Leo self-review"** section — this is where §15's "Leo posts its self-review artifact as an actual PR comment thread" actually lands: the same Self-Review artifact from §13.2 gets posted as real PR comments through this panel's GitHub connection, and also renders natively here so you don't have to leave Spartan to read what Leo already told GitHub
- **Create PR** from the current branch: pre-fills a description from the branch's commits and, if the session has one, the originating `ImplementationPlan` artifact's summary — editable before posting, never auto-submitted
- **Checks tab**: per-check status pulled from the GitHub Checks API, a failing check's log excerpt surfaced inline with the same "ask Leo about this" affordance as any other log line (§14)

### 56.4 GitHub Layer — Issues

- Read/search issues for the current repo inline (title, labels, assignee), link one to the active session — this is the same two-way issue-tracker sync concept §26 describes for Jira/Linear, GitHub Issues just being the zero-additional-setup case when the repo already lives on GitHub
- An `ImplementationPlan` artifact can be converted into a tracked issue on approval (§26's existing "close the loop" design), and a session started *from* an issue pre-fills Leo's initial context with the issue body

### 56.5 Leo Tool Belt

| Tool | Behavior |
|---|---|
| `git_status` / `git_diff` | read-only, always allowed |
| `git_commit` | approval-gated per §4.1's matrix, same as any file-write action |
| `git_branch_create` / `git_checkout` | approval-gated if the target would discard uncommitted work, otherwise low-friction |
| `github_list_prs` / `github_get_pr` | read-only |
| `github_create_pr` | approval-gated — Leo can prepare a PR description as part of a plan, never opens one silently |
| `github_post_review_comment` | specifically how §15's self-review-to-PR-comment flow is implemented; still subject to the same approval posture as any externally-visible action |
| `github_list_issues` / `github_link_issue` | read-only / low-friction linking |

### 56.6 Tier Placement

§35.4's Tier 1 table never had a row for this, which was itself part of the gap — added now: **basic local Source Control (§56.1) is Tier 1**, table-stakes for the same reason Phase 5 already implied it (a git-less IDE isn't a credible daily driver). The **GitHub-specific layer (§56.2–§56.4)** is Tier 1 for read-only PR/issue visibility and PR creation, since that's cheap relative to the value and directly enables §15's PR pre-review feature; deeper GitHub Actions log parsing and multi-provider (GitLab/Bitbucket) parity are Tier 2, consistent with §35.5's general sequencing of "make the common case excellent before generalizing."

---

## 57. LM Studio — A Second Local Runtime (amends §3.3, §41)

### 57.1 Why a Second Hand-Rolled Provider, Not LiteLLM

CLAUDE.md's own rule is explicit: new providers go through LiteLLM unless there's a specific reason to hand-roll, "as already decided for `ClaudeProvider`/`OllamaProvider`." LM Studio is being added as a third named exception, for the same category of reason `OllamaProvider` was: the differentiated value isn't the chat-completion call itself (LiteLLM could proxy that trivially, since LM Studio speaks an OpenAI-compatible REST API) — it's **local model lifecycle management** woven into the Local Model Manager (§3.3, §41): detecting what's installed, showing disk usage, surfacing pull/load progress, and reconciling two different local runtimes' installed-model lists into one UI. That integration depth is exactly what justified writing `OllamaProvider` by hand instead of proxying it, and the same logic applies here rather than being a new, un-argued exception.

### 57.2 LmStudioProvider

- Talks to `http://localhost:1234/v1` by default (LM Studio's local server mode), configurable for a remote LM Studio instance the same way `OllamaProvider` supports a non-localhost Ollama host (§3.3)
- **Startup detection**: pings `/v1/models` on launch; unreachable means LM Studio options gray out in the model picker, same graceful-absence posture as Ollama — neither local runtime is a forced dependency
- Native OpenAI-compatible tool-calling where the loaded model supports it; falls back to the same structured-output fallback scheme (§3.4) for models that don't, since that scheme was already written model-agnostic rather than Ollama-specific
- Context window read from LM Studio's model metadata endpoint rather than hardcoded, mirroring §3.3's `/api/show`-based approach for Ollama

### 57.3 One Local Model Manager, Two Runtimes

Rather than a second, parallel model-management UI, the existing Local Model Manager (§3.3, §41) gains a **runtime** dimension:

- The **Installed** tab's rows carry a runtime badge (Ollama / LM Studio) since the same GGUF model can be loaded into either — showing both prevents the confusing "why does Spartan think I don't have this model" case when it's sitting in the other runtime's directory
- **Curated** and **Hugging Face** tab pulls specify which runtime to pull into (a model already available in one is offered in the other rather than hidden), since a user may prefer LM Studio's own GUI for model management but want Spartan's curated picks, or vice versa
- Routing settings (§3.5) treat `LmStudioProvider` as an equal local option alongside `OllamaProvider` for `Hybrid`/`LocalOnly`/`PrivacyScoped` modes — nothing in the routing engine special-cases which local runtime is active

### 57.4 What This Does Not Change

LM Studio does not get its own curated-model manifest (§3.3's manifest is runtime-agnostic — a model recommendation is a model recommendation regardless of which local server ends up running it), and it does not get a separate settings category — it lives inside Leo & Models (§42) as a second connected-provider row and a runtime option in the same Local Model Manager, not a parallel settings surface.

---

## 58. API Keys & Credentials — A Dedicated Settings Category

Secrets handling was already specified piecemeal — the OS keychain principle (§27), Hugging Face tokens (§41.4), LiteLLM virtual keys (§44.5) — but scattered across sections a user would have no reason to think to check when they just want to "put in my Anthropic key." This gives it one findable home, per §42's settings taxonomy.

### 58.1 What Lives Here

A new top-level settings category, **API Keys & Credentials**, distinct from Leo & Models (provider *routing*) and Privacy & Security (vault *backend choice*) — this page is the actual list of individual credentials:

| Credential | Source | Actions |
|---|---|---|
| Anthropic API key (direct/BYOK) | User-entered or reused from the desktop app's own auth | Add, rotate, remove, test-connection |
| Per-provider LiteLLM keys (OpenAI, Azure, Bedrock, etc.) | §44.5's fallback-chain editor feeds from here | Add, remove, scope-cap display (spend limit if set) |
| GitHub | §56.2's device-code flow | Reconnect, view granted scopes, revoke |
| Hugging Face token | §41.4 | Add, remove (gates private/gated model pulls) |
| Custom (MCP servers, plugins) | Registered by a plugin's `settings.register_page` hook (§42.4) or added manually | Add, remove, scope label showing which plugin/MCP server owns it |

### 58.2 Design

- Every value is **masked by default** (`sk-ant-••••••1a2b`), never rendered in full without an explicit "reveal" click that itself requires re-authentication (OS-level, e.g. Touch ID/Windows Hello) if the platform supports it
- Storage is exactly §27/§41.4's existing rule, unified here rather than re-decided per-credential: OS keychain, never plaintext in `config.toml`, never sent to Leo's context window, and specifically excluded from the secrets-scanning-before-cloud-context pass (§9) because it should never have been in scanned content to begin with — the vault, not the scanner, is the actual boundary
- Each row shows **last used** (timestamp + which subsystem: "used by LiteLLM routing, 2h ago") — the same auditability principle as §41.7's model-provenance logging, applied to credentials instead of models
- Adding a key that matches a known provider's format runs an immediate, real **test-connection** call before saving — catching a copy-paste error (truncated key, wrong env var pasted) at entry time instead of at first real use mid-task
- Removing a credential that's actively referenced by a routing rule, fallback chain, or MCP connection warns which of those break first, rather than silently leaving them pointing at nothing

### 58.3 What This Does Not Change

No new storage mechanism, no new trust model — this is a UI consolidation of credential management that already had a correct backend design (§27) but no single page a user would find it under. Per §42.3's rule, adding or rotating a credential is itself a settings change with full change-history/revert support, and removing a GitHub or cloud-provider connection is treated with the same "security-relevant change" confirmation weight as widening an approval scope (§42.1).

---

## 59. Terminal Panel — Filling a Gap Left Open Since §14

"The embedded terminal panel (§14)" is cited twice — §46.1's CLI rationale, §19's terminal-copilot grab-bag idea — and §14 itself ("Additional Developer Tooling") never actually described a terminal panel at all, only adjacent tooling (profiler, DB explorer, log viewer). Same failure mode §56 opened with for the git panel, caught the same way: by actually checking rather than assuming the citation pointed somewhere real.

### 59.1 What It Is

A first-class dockable panel (same shell as the Debug Console, Device Panel, and Source Control dock, §21.3/§32/§56.1) wrapping one or more real PTY sessions:

- **Multi-tab**: each tab is an independent shell session (default shell auto-detected: zsh/bash/fish on Unix, PowerShell/cmd on Windows, §61 covers WSL as an additional tab *kind*), renamed, closable, reorderable
- **Split panes** within a tab group for side-by-side output watching (e.g., a dev server log next to an interactive shell) — a simple horizontal/vertical split, not a full tiling-window-manager scope
- Full ANSI color, real terminal resize (`SIGWINCH` propagated correctly, not a fixed-size text box), scrollback search
- **Persistent across sessions** per the named-layout system (§16.2, §63) — reopening a project can restore which terminal tabs were open, not just which files

### 59.2 The Important Distinction This Section Exists to State Clearly

A human typing directly into this panel is running a **real, unsandboxed shell** — exactly like opening a normal OS terminal, because that is what this panel is. It is not, and must never be presented as, the same trust boundary as Leo's `run_terminal` tool (§4.5), which *is* sandboxed (working-directory jail, environment allowlist, timeout/output caps, approval-gated). Conflating the two would be a real, dangerous misunderstanding of the security model: a user typing `rm -rf` themselves in this panel is not something Spartan's approval system is meant to catch or should try to intercept — that would be a surprising, unwanted nanny-mode for a tool that is supposed to be a real terminal. The approval gating in §4.5/§9 governs what *Leo* does with this same PTY infrastructure when Leo is the one issuing commands (e.g., an agent step that opens a terminal tab to run tests), not what the human does by hand in a tab they opened themselves.

### 59.3 Terminal AI Co-Pilot (absorbing §19's grab-bag idea)

- Natural-language-to-shell-command inline: typing a `#`-prefixed comment-style query (`# find all TODO comments added in the last week`) suggests a real shell command as ghost text, ready to accept or edit — never auto-executed
- **Dry-run preview** for anything the co-pilot suggests that looks destructive (matches the same destructive-command heuristics §9/§36.4 use elsewhere) — shows what the command would affect before the user confirms running it
- `spartan explain-last` (§46.5) and "ask Leo about this output" both work from any terminal tab, not just the ones Leo itself opened, since the human's own terminal output is just as valid a source of "what just happened" context

### 59.4 Tier Placement

Tier 1 — same reasoning as §56.6 gave the Source Control panel: a daily-driver IDE without a working terminal is not a credible v1, and this was always implicitly assumed in scope by the two sections that cited it. Split panes and full named-layout persistence (§59.1) can slip to v2 if needed without weakening the v1 bar; a single-tab, no-split terminal would still clear it.

---

## 60. Developer Mode — Explicit, Scoped, Never Silent (amends §9, §36.4.6, §42)

### 60.1 What This Is Not

Framed first, because it's the part most likely to be gotten wrong by anyone implementing this from the feature name alone: "Developer Mode" is **not** a switch that disables destructive-action approval or plugin sandboxing. §9 is explicit that destructive actions are never trusted "without explicit approval, regardless of routing mode or autonomy setting" — that floor holds even in Developer Mode, revised scope and all (§60.2.1). What Developer Mode *does* now widen, per an explicit revision made with the tradeoff stated plainly and a deliberate choice made (§60.2.1), is the path-jailing boundary — §36.4.6 is amended accordingly, not silently contradicted. A toggle that quietly disabled destructive-action approval to satisfy a feature request would be exactly the kind of failure §36 catalogs in other tools; that line is the one thing this section will not move.

### 60.2 What It Actually Relaxes

| Relaxed | Mechanism | Still true while active |
|---|---|---|
| Working-directory scope | **Revised (§60.2.1): no path allowlist at all** — any path the OS user account can reach, not jailed to an expanded set | The *first* action that would write outside the original project directory still surfaces one explicit confirmation, then is remembered for the rest of the session — never zero confirmations, ever, for the first boundary crossing |
| Environment/secrets passthrough to Leo's terminal tool | §4.5's existing per-session opt-in ("NOT auto-injected unless explicitly permitted per-session") becomes a standing setting instead of a per-session ask | Still visible per-call in the tool-execution log; still revocable instantly |
| External content fetch gating (§50.2) | Relaxed from per-fetch approval to a standing allow for this project | Still logged; still excluded from cloud context if the secrets-scanning pass would otherwise flag it |
| Per-command approval for reads, edits, and non-destructive shell commands | No approval prompt at all while Developer Mode is active — equivalent to Autonomous/Vibe (§4.1, §45) but scoped to this workspace rather than a global autonomy setting | Destructive actions (`rm -rf`, `git push --force`, migrations, `sudo`) still require one explicit approval per §9 — this is the one gate Developer Mode never touches, at any revision |

### 60.2.1 Revision — Widened at Explicit User Request, Documented Rather Than Silently Changed

The original version of this section (written the same session §56–§62 were added) scoped path-jailing to an expanded-but-still-jailed allowlist, reasoning that §36.4.6's "regardless of what the model requests" invariant shouldn't have a feature-shaped exception carved out without a very deliberate reason. A later request asked explicitly for wider access, framed as "unsandboxed full access actions." Rather than either silently implementing the literal phrase (which would have removed the destructive-action floor too — a materially different and more dangerous change) or refusing outright, the tradeoff was surfaced directly and the user chose the specific scope now reflected in §60.2's table: no path jail at all once Developer Mode is on, but destructive-action approval and a first-boundary-crossing confirmation both stay. This is the same shape as real shipped precedent — Claude Code's `--dangerously-skip-permissions`, Cursor's YOLO mode, Aider's `--yes` — wide open on the "does this need a prompt every time" axis, with a floor still under the specifically irreversible axis. §36.4.6 is amended to name this as the one documented exception to its invariant, rather than leaving the two sections contradicting each other.

### 60.3 Enforcement of "Never Silent"

- Enabling it requires the full **security-relevant change confirmation** flow (§42.1/§42.3) — not a casual toggle-and-forget — showing exactly which rows in §60.2 will change and what stays hard-enforced regardless, updated to reflect §60.2.1's widened path scope
- While active, a **persistent, un-dismissable indicator** (not just a settings-page toggle state) stays visible in the top rail for the whole session — the same "never let the user forget an override is live" posture as §3.5's PrivacyScoped lock badge
- Every relaxed action taken while Developer Mode is active is tagged distinctly in the session's append-only audit log (§7, §18), and the first out-of-project-directory write is tagged with extra prominence — a later "why did this touch a file outside the project" question has a direct, honest answer with a specific moment it was confirmed
- Off by default, every time — this is deliberately **not sticky across projects**; opening a different project with Developer Mode enabled on another project does not carry it over, since the whole point is that the user consciously chose the wider trust boundary *for this specific workspace*

### 60.4 Tier Placement

Tier 2 — genuinely useful for the power-user/monorepo-adjacent-repo case it targets, but not required for the Tier 1 "credible daily driver" bar, and worth shipping only after the core approval-gate machinery (§4.1, §4.5) it layers on top of has real usage behind it.

---

## 61. WSL & WSA — Windows Subsystem Integration (amends §20, §21, §33, §37.1)

### 61.1 WSL (Windows Subsystem for Linux)

On Windows, a WSL distro is a legitimate, common dev target — not an edge case. Rather than a bespoke subsystem, WSL slots into the **Virtual File System abstraction §37.1 already specifies** ("local, SSH-remote, container... all present through the same file-tree API") as one more backend: a detected WSL distro (`wsl -l -v` equivalent) is offered as a workspace root exactly like a container or SSH remote, with the same file-tree/task-runner/terminal experience, not a separate mode with its own UI to learn.

- **Detection**: on launch, Spartan checks for WSL availability and lists installed distros, the same graceful-absence pattern as Ollama/LM Studio (§3.3, §57.2) — no WSL installed just means the option doesn't appear, never an error
- **Toolchain resolution inside WSL**: language profiles (§20.1) auto-detect and resolve toolchains *inside* the selected distro (e.g., a Linux `rustc`/`node` that differs from the Windows host's), the same way a container or SSH-remote toolchain already resolves independently of the host machine per §37.1
- **Terminal tabs** (§59.1) gain a "WSL: <distro>" tab kind, a real PTY into `wsl.exe -d <distro>`, subject to the same human-vs-Leo trust distinction §59.2 draws — a WSL terminal tab is exactly as unsandboxed for direct human use as any other tab
- **Cross-boundary file performance**: Spartan surfaces (rather than silently eating) the well-known WSL performance cliff of accessing Windows-drive paths (`/mnt/c/...`) from Linux tooling — a one-time advisory when a WSL-backed project root resolves onto a Windows drive, suggesting the project live inside the Linux filesystem instead, consistent with never hiding a footgun the user could easily avoid if told about it (§36's general posture)

### 61.2 WSA (Windows Subsystem for Android)

WSA is not a new subsystem either — it's a third **device kind** in the existing Device Panel (§21.3, §33.1), alongside physical devices and emulators, since WSA exposes the same ADB debug bridge those already use:

- Auto-detected the same way physical/virtual devices are (`adb devices` sees it once WSA's developer mode + ADB debugging are enabled in the WSA settings app) — Spartan doesn't launch or configure WSA itself, since that's Windows' own settings surface, not Spartan's to own
- Everything already specified for devices applies unchanged: Files/Shell/Logcat/Processes/Screen/Performance/Package Manager tabs (§33), the Screen Mirror panel (§33.7), on-device JDWP debugging (§21.3) — WSA is just a `device_kind: "wsa"` tag, not a parallel feature set to build and maintain
- One real difference worth calling out rather than glossing over: WSA (unlike a physical device or standard emulator) runs under Hyper-V/WSL's own virtualization, so its own performance characteristics and occasional GPU-passthrough quirks are Microsoft's to fix, not Spartan's — the Device Panel labels a WSA connection as such (not silently presented identically to a Pixel emulator) so a user debugging a graphics-rendering oddity knows to consider that variable

### 61.3 Tier Placement

Tier 2 — Windows-specific breadth on top of an already-committed Tier 1 Android/device story (§35.4); valuable but not gating v1, which ships with physical-device and standard-emulator support first.

---

## 62. Slash Commands & Panel Visibility (amends §8, §16.1, §16.2)

Two smaller interface additions, grouped because both are "make the existing interaction model more discoverable/controllable" rather than new subsystems.

### 62.1 Slash Commands in the Agent Chat Input

Distinct from the global ⌘K command palette (§16.1) — that's a modal, app-wide entry point; this is inline, scoped to the chat input the user is already typing in, matching the pattern Slack/Discord/Claude.ai already trained users on. Typing `/` at the start of the Agent View input opens an inline dropdown, filtered as more characters are typed, with no modal takeover:

| Command | Behavior |
|---|---|
| `/plan` | Ask Leo to produce an `ImplementationPlan` artifact before touching any file, without needing to phrase it as a sentence |
| `/test` | Runs the project's configured test task (§20.2), same as asking in prose, just faster to invoke |
| `/commit` | Opens the Source Control panel's commit view (§56.1) pre-focused on the message box |
| `/pr` | Opens the Pull Request tab (§56.3), starting the create-PR flow if the branch has no open PR yet |
| `/explain` | Equivalent to selecting code and asking "explain this" — usable with no selection to mean "explain what's currently on screen" |
| `/model` | Jumps to the Leo & Models settings category (§42.2) model picker, for a fast mid-session switch |
| `/vibe` | Starts the current task in Vibe Mode (§45) instead of the session's default autonomy level, for this task only |
| `/undo` | Requests rollback to the last checkpoint (§4.2) — a fast path to the same real, tested rollback the left rail's session history already exposes |
| `/clear` | Clears the visible chat transcript for this session (does not delete the underlying append-only session log, §7 — display-only, matching "nothing is silently destroyed") |

Every slash command is implemented as a thin dispatcher onto an *existing* mechanism (a tool call, a settings jump, a panel focus) — this section adds no new backend capability, only a faster on-ramp to ones already specified elsewhere, which is also why the list is easy to extend: a new command is only ever a new mapping, never new plumbing.

### 62.2 Panel Visibility

§16.2 already specifies named layouts and a Focus Mode that collapses "everything but the active file." What was missing is a direct, per-panel visibility control independent of switching an entire named layout — sometimes a user wants to hide just the Auxiliary Pane, not adopt a whole different layout preset:

- Every dockable surface (Left Rail, Auxiliary Pane, the Editor View's docked panel group from §59/§33/§56, a docked Leo mini-panel) gets a consistent hide affordance in the same visual location, not a different interaction per panel
- A single **View** control in the top rail lists every currently-available panel with a checkbox-style toggle — one place to see "what's currently hidden," rather than having to remember which panels have their own individual close buttons scattered around the shell
- Hidden state is part of the same session/workspace state a named layout captures (§16.2) — closing the Auxiliary Pane and reopening the project later remembers that choice, the same durability already promised for layout presets
- This is explicitly **visibility**, not **removal** — a hidden panel's underlying state (an in-progress commit message, an open terminal tab) is preserved and restored on reveal, not torn down, consistent with the project's "nothing silently discarded" thread running through §36 and the harness-level git safety rules this project's own tooling already follows

---

## 63. Skills — Lightweight Agent Capability Packages (amends §5, §34.7, §55)

§55's parity matrix, folding the legacy console's AI skills registry into this architecture, left a forward reference unresolved: "a skill here is functionally a named tool/command bundle, which §5.3's `agent_api` extension point already generalizes." That's true as far as it goes, but it under-specifies the actual authoring/import experience — §5 is deliberately heavyweight (compiled WASM, capability-sandboxed, marketplace-vetted), which is the right bar for a plugin that can register new tools, but the wrong bar for what most people mean by "a skill": a folder of instructions, maybe a script, that teaches Leo how to do one specific thing well.

### 63.1 What a Skill Is, Distinct From a Plugin

A skill is a directory with a manifest (`SKILL.md` — a markdown file with frontmatter: name, description, when-to-trigger hints) plus optional supporting scripts/resources, loaded directly into Leo's context when relevant rather than compiled and capability-sandboxed like a WASM plugin (§5). The distinction matters architecturally, not just semantically:

| | Skill | WASM Plugin (§5) |
|---|---|---|
| Format | Markdown + optional scripts, interpreted | Compiled WASM, capability-sandboxed |
| Registers new tools? | No — teaches Leo how to use *existing* tools better for a specific task | Yes, via `agent_api` (§5.3) |
| Trust model | Read as context, same secrets-redaction pass as any file (§9) before reaching a cloud provider | Explicit capability grant per §5.2, enforced at the import-binding level |
| Authoring bar | Anyone who can write a clear markdown doc | Requires a WASM-targetable language toolchain |
| Example | "How this team writes database migrations," "our commit message convention," "how to debug the flaky checkout test" | A linter bridge, a theme, a Jira integration that needs to make real API calls |

### 63.2 Skills Settings — Import & Manage

A new **Skills** settings category (§42.2):

- **Installed** tab: every skill currently active for this project or globally, with its trigger description, source (local folder / imported bundle / marketplace), and an enable/disable toggle per skill without needing to delete it
- **Import** flow: point at a local folder, a `.zip`/tarball bundle, or (mirroring §41.4's "reuse established tooling" posture) a git URL to clone from — each import is validated against the `SKILL.md` schema before activation, with a clear error rather than a silent no-op if the manifest is malformed
- **Marketplace** tab, reusing the exact same trust/verification presentation as the WASM plugin marketplace (§5.4) — signed bundles, a "built from public source, verified" badge — since a skill can still smuggle in prompt-injection-shaped content even without code execution capability, the same scrutiny applies to *content* that a plugin's *capability grant* gets
- **Scope**: Project (`.spartan/skills/`, git-committable and shareable with a team the same way project memory is, §4.3) vs. Global (`~/.spartan/skills/`) — a skill can be promoted from global to project scope or vice versa from this panel

### 63.3 How Leo Uses a Skill

- Skill manifests' trigger descriptions are indexed the same way project memory is summarized into context (§4.3) — a background relevance pass surfaces the right skill for the current task rather than dumping every installed skill's full content into every turn's context, which would blow the token budget for no benefit
- When a skill's script component runs (not just its instructional markdown), it goes through the exact same sandboxed subprocess model as any other tool call (§4.5) — a skill with a script is not a backdoor around the tool-execution sandbox just because it arrived as a "skill" instead of a plugin
- Skills compose with the existing Team Memory tier (§4.3, §15): a team's shared skills and shared memory both live in `.spartan/`, git-committed, reviewed like code

### 63.4 Tier Placement

Tier 2 — genuinely useful once there's real usage data on which repeated tasks are worth codifying as a skill, and depends on §4.3's memory system and §5's capability model both already existing to slot into cleanly.

---

## 64. MCP Server Management Panel (amends §36.4.6, §46.3, §58)

§58's credential settings had a row for "Custom (MCP servers, plugins)" credentials, and §36.4.6 already establishes that registering a new MCP server is a security-relevant, approval-gated action — but neither specifies the actual management surface for MCP *connections* themselves (transport, command, capabilities exposed), as distinct from the credentials one might use with them. This section is that surface.

### 64.1 What Lives Here

A dedicated **MCP Servers** settings category (§42.2), separate from API Keys & Credentials (§58) the same way Leo & Models' provider *connections* are separate from the *keys* those connections use:

- **Connected servers** list: name, transport (stdio / SSE / streamable HTTP), status (connected / unreachable / awaiting approval), and the capability surface each one actually exposes once connected (tool list, resource list) — shown explicitly rather than treated as an opaque black box once approved
- **Add server**: stdio command + args, or a URL for SSE/HTTP transports — mirrors `spartan mcp serve`'s own shape (§46.3) so configuring an inbound and outbound MCP connection feels like the same mental model, not two unrelated features
- Every add/edit is the security-relevant diff §36.4.6 already requires approval for — this panel is the UI that diff renders in, not a new trust decision bolted alongside the existing one
- **Per-server tool allowlist**: same granularity as Fleet's per-tool permissioning (§52) — a connected MCP server's tools can be individually allowed/denied rather than all-or-nothing, so a project-management MCP server's read tools can be trusted while its write tools stay approval-gated
- **Health check**: matches the Doctor diagnostic pattern (§42.4) — one-click reachability/latency check per server, surfaced inline rather than only discovered when a tool call silently times out mid-task

### 64.2 Relationship to Open Design's MCP Interoperability (§38)

§38 already describes Leo registering an Open Design MCP server as a tool source, and Spartan exposing its own project via a compatible MCP endpoint. This panel is the general-purpose home for that specific case too — Open Design's connection is one row in this same list, not a special-cased separate settings surface, consistent with §57.4's "no parallel settings surface" principle already applied to LM Studio.

### 64.3 Tier Placement

Tier 1 for the basic connect/approve/list surface — MCP connectivity is already assumed elsewhere in the spec (§38, §46.3) and needs *some* management UI to be usable at all; the per-tool allowlist granularity and health-check diagnostics can slip to Tier 2 without blocking v1's basic "add and use an MCP server" case.

---

## 65. Playwright Integration — Live Testing & Visual Debugging (amends §24, §32, §6)

§24 already lists Playwright as one of many auto-discovered test frameworks, and mentions visual regression tied to Design View's canvas. What's missing is Playwright as a *live, driveable* surface Leo and the user both reach for directly — not just a test runner whose results appear in a tree.

### 65.1 Live Browser Panel

- A dockable panel (same shell as the Terminal, Source Control, and Device panels — §59, §56, §33) embedding a real, visible browser instance driven by Playwright, not a screenshot-on-a-timer — page navigation, DOM state, and console output are all live
- **Click-to-inspect**: clicking any element in the live panel surfaces its selector, computed styles, and accessibility tree node — feeding directly into Leo's context the same way a stack trace or log line does elsewhere (§14), so "why does this button look wrong" can be answered by clicking it, not describing it in prose
- **Record-to-test**: interactions performed in the live panel are recorded as a real Playwright test script in the project's language/config, editable before saving — the same "generate, then let a human or Leo review" posture as any other codegen path in this spec (§6.2's `CanvasEdit` → real diff, §34's design-to-code)

### 65.2 Visual Debugging

- **Screenshot diffing** extends §24's visual regression: a failing visual test's pixel diff renders inline in this panel exactly as it would in Design View's canvas (§24 already established this tie for the canvas case; this section is the same diff view reachable from a live, currently-running page instead of only a completed test run)
- **Trace viewer**: Playwright's own trace format (DOM snapshots, network, console, per-action screenshots) opens inline rather than requiring a separate `npx playwright show-trace` — a failing CI run's trace artifact can be pulled and inspected without leaving Spartan
- **Leo's self-verification loop, extended to the browser**: for a UI change, Leo can drive the live panel itself as part of its Verify phase (§4.1) — navigate to the changed page, confirm the expected element renders/behaves as intended, and attach the resulting screenshot to the diff artifact as verification evidence, the same self-verification pattern §21.6 already establishes for `compose_preview_render` on Android, now with a web equivalent

### 65.3 Security Posture

The live browser panel is a real, capable automation surface — network access, arbitrary page navigation, script execution in-page — so it inherits the exact same sandboxing posture as any other tool Leo can drive (§4.5): default-scoped to localhost/the project's own dev server unless the user explicitly permits external navigation, and any Leo-initiated navigation to a non-localhost origin is treated as an external-content action subject to the fetch gating settings (§50.2, and Developer Mode's relaxation of it per §60.2 where applicable).

### 65.4 Tier Placement

Tier 2 — the universal test explorer and basic visual regression from §24 are enough for a credible Tier 1 Test Studio; the live/driveable panel and Leo's browser-based self-verification loop are a meaningful upgrade on top, not a blocker.

---

## 66. CPU Render Fallback (amends §2.2, §39.1, §47.1)

§47.1 was honest that Spike 0.1's GPU half "has never run — no display/GPU in the environment this was built in," across every session so far. That gap in what could be *validated* pointed at a gap in what the *design* accounted for: §2.2's rendering pipeline assumed a GPU is always available, with no stated fallback for the real population of machines that don't have one — remote dev boxes, CI runners taking a screenshot for a bug report, cheap cloud VMs, or a display driver that simply fails to initialize.

### 66.1 Detection & Fallback Path

- On startup, Spartan probes for a usable `wgpu` backend (Vulkan/Metal/DX12/GL) the same way it probes for Ollama/LM Studio/WSL — gracefully, not as a hard requirement (§3.3, §57.2, §61.1's shared pattern)
- If none is available, or if GPU initialization fails partway through (a real, not just theoretical, failure mode — GPU drivers do crash), Spartan falls back to a **CPU software rasterizer** path rather than refusing to start: the same glyph-atlas/SDF rendering model from §2.2, rasterized on CPU (via `wgpu`'s own software backend, e.g. Lavapipe/SwiftShader, rather than a hand-rolled second renderer — reuse, don't rebuild, per the standing principle already applied to scrcpy in §35.7 and llama.cpp conversion in §41.4)
- The fallback is visible, not silent: a persistent, dismissible indicator states "Software rendering — GPU unavailable," with a link to what that means for performance, rather than the user wondering why the editor feels different with no explanation

### 66.2 Performance Envelope, Honestly Stated

- CPU rendering will not hit §10's <5ms p99 keystroke-to-glyph target — this section does not pretend otherwise. The realistic target for the fallback path is "fully usable for editing and agent work, visibly slower for rapid scrolling/animation," with motion/animation effects (§8.4's spring-curve plan tracker, fade transitions) automatically simplified or disabled under CPU rendering the same way `prefers-reduced-motion` already disables them (§16.3) — a CPU-rendering user gets the reduced-motion treatment by default, overridable if they'd rather have the animations at a performance cost they've accepted
- Damage-region rendering (§2.2) matters *more*, not less, under CPU fallback — re-rasterizing only the changed viewport region is the single highest-leverage optimization available without a GPU, so this is where fallback-path engineering effort concentrates rather than chasing full frame-rate parity that CPU rendering can't realistically hit

### 66.3 Manual Override

A **Renderer** setting (Appearance category, §42.2) lets a user force CPU rendering even with a working GPU present — legitimate for remote-desktop/VDI sessions where GPU passthrough is unreliable, screen recording setups where GPU-accelerated compositing fights with capture software, or simply diagnosing whether a rendering glitch is GPU-driver-specific. Forcing GPU rendering when none is detected is not offered as an option — that would just be a slower way to reach the same crash the auto-detection already exists to avoid.

### 66.4 Tier Placement

Tier 1 — this is not a nice-to-have edge case; it's the difference between the editor starting at all on a real slice of target hardware (remote dev boxes, CI, cheap VMs) versus refusing to launch. §39.1's spike should validate the CPU path's performance envelope with the same rigor already asked of the GPU path, not treat it as an afterthought bolted on after GPU-only development.

---

## 67. Google Antigravity 2.0 Feature Parity Matrix (amends §8, §22, §36.2, §50)

§50 already fact-checked several specific Antigravity 2.0 claims against real sources rather than memory (the Workspaces+Playground+Inbox structure, the Editor/Manager keyboard split, Fast/Planning mode, the external-content-fetch vulnerability). A later request asked to go further — integrate the whole feature set. Rather than treat that as license to fabricate a feature list from training-data recall, it was researched fresh this pass, the same discipline §50 already established. This section is the systematic result: every real, documented Antigravity 2.0 feature found, mapped to its Spartan home, its gap, or its deliberate non-match — the same traceability-matrix pattern §55 already uses for the legacy console's features, applied to a competitor's feature set instead of this project's own prior product.

### 67.1 Methodology

Sources: Google's own developer blog and Codelabs announcement, TechCrunch's I/O 2026 coverage, Antigravity's own settings/design documentation, and — for the parts of the request that were specifically about UI simplification — two independent user-reported accounts of a real 2.0-era usability regression (cited already in §50.3 and §36.2). Each feature below is marked one of four ways: **Matched** (Spartan's existing spec already covers the equivalent), **Exceeded** (Spartan's architecture already does more than the Antigravity equivalent), **Deliberately Different** (Spartan's own locked architectural decisions make 1:1 parity impossible or undesirable, named rather than silently skipped), or **Gap Closed This Pass** (a real, concrete addition made specifically because this research surfaced it).

### 67.2 Parity Matrix

| Antigravity 2.0 Feature | Spartan Status | Where |
|---|---|---|
| Agent Manager / Manager Surface — spawn, orchestrate, observe multiple agents across workspaces, inbox-styled task list | **Matched** | §8, §50.1 (`WORKSPACES` strip + Playground + Inbox, already amended a previous pass) |
| Editor View — AI-powered IDE, tab completions, inline commands | **Matched** | §2, §4 |
| Editor/Manager keyboard-level split (Antigravity's `Cmd+E` equivalent between agent-only and combined views) | **Matched** | §8, §50.1 — already a confirmed design decision, not newly added |
| Fast mode vs. Planning mode, explicit per-task toggle | **Matched** | §42.2, §50.1. Reports on whether 2.0 kept this exact toggle are mixed between sources — Spartan keeps its own regardless, since it's Spartan's design decision independent of Antigravity's current state |
| Scheduled tasks for background automation | **Matched** | Already visible in the reference prototype's Inbox rail ("Dependency audit · 9:00 AM daily") |
| Async task delegation for long-running work (maintenance, bug fixes) | **Exceeded** | §4's autonomy-level model (Manual/Plan-Approve/Autonomous/Vibe, §45) already covers this generally, not as a separate bolted-on "async mode" |
| Model optionality (Gemini 3 Pro, Claude Sonnet 4.5, GPT-OSS side by side) | **Exceeded** | §3/§44 — any LiteLLM-fronted model plus first-class Claude and local Ollama/LM Studio, a broader set than three named models |
| Cross-platform (macOS, Windows, Linux) | **Matched** | Stated at the README/§1 level since this project's founding pass |
| `DESIGN.md`-style design-system doc, read automatically by the agent | **Matched** | §6, §38 (Open Design integration) — independently arrived at, confirmed convergent rather than copied |
| Browser Subagent — agent autonomously drives a real browser instance to click, fill, test, and self-fix without the user opening devtools | **Gap Closed This Pass** | §65 amended: Leo can now drive the same Live Browser panel autonomously as part of its own verification loop (mirrors §21.6's existing Android compose-preview pattern), not just the user manually clicking around. Real toggle added to the reference prototype's `PlaywrightPanel` |
| Artifacts (plans, task lists, screenshots, browser recordings) commentable directly, like a doc | **Gap Closed This Pass** | Every `ArtifactShell` in the reference prototype now has a comment affordance — feedback attaches to the specific artifact, not a separate chat message about it |
| Knowledge base — agents save reusable context/snippets across sessions | **Matched** | Covered by the existing Memory settings category and §7's data model; not a new gap |
| CLI for headless agent creation, no GUI required | **Matched** | §46 (Spartan CLI) |
| SDK for programmatic, self-hosted agent definition | **Honest Remaining Gap** | Not yet spec'd. Noted rather than silently claimed — see §67.5 |
| Live voice transcription | **Honest Remaining Gap** | Not yet spec'd. See §67.5 |
| VS Code extension compatibility ("most extensions still functioning") | **Deliberately Different, partially bridged this pass** | Running arbitrary extension code as-is is impossible without the VS Code/Monaco extension host this project has locked out from day one (CLAUDE.md) — that stays true. What's real now: §68's manifest importer converts an extension's static `contributes` declarations (commands, keybindings, simple config, snippets) into a real Spartan WASM Plugin (§5); anything needing the actual VS Code API surface is explicitly rejected at import time, not silently no-op'd |
| VS Code theme ecosystem ("vast ecosystem of...themes") | **Gap Closed This Pass** | Raw VS Code theme JSON can't run as-is for the same reason above — added a VS Code theme **importer** (converts into Spartan's own token model) to Appearance settings instead of a dishonest compatibility claim |
| High-contrast, minimal visual theme | **Matched, this pass** | §50.3 — palette tokens tightened to Antigravity's researched values, one named divergence (accent hue) kept for identity reasons |
| External Content Fetch Gating vulnerability class | **Matched** | Already closed, §50.2 |
| 2.0-era minimalism regression (removed terminal, inline diagnostics, direct-edit code view) | **Deliberately Not Replicated** | §36.2's new failure-catalog row, §36.4.10's permanent boundary |

### 67.3 What "Gap Closed This Pass" Actually Means Here

Consistent with this document's standing rule against overclaiming: these are reference-prototype UI additions and spec amendments, not shipped product code — the same honest framing as everything else in `interface-prototype.jsx` (§47/§48/§51's "reference-only, not implemented" line applies here too). What changed is real and verified (parse-checked, rebuilt, and exercised via Playwright with zero console errors — the Leo-driving toggle and artifact-comment flow were both actually clicked through, not just described), but it's still a mockup demonstrating the interaction design, not Leo's actual agent loop or a real browser-driving implementation.

### 67.4 Honest Remaining Gaps

Two items have no home yet and are named as open rather than silently absorbed into an existing section to make the matrix look more complete than it is:

- **A programmatic/self-hosted agent SDK** (Antigravity's equivalent lets third parties define custom agent behaviors and host them independently). Spartan's CLI (§46) covers headless *use*; nothing yet covers headless *extension* at the SDK level. Tentatively Tier 3 — behind Tier 0's remaining spikes and Tier 1's MVP scope per §35 — not something to spec in depth opportunistically inside this matrix.
- **Live voice transcription** as an input method. No existing section covers voice input at all. Tentatively Tier 2, sitting alongside §16's other interface enhancements — worth its own proper design pass rather than a rushed paragraph here, since input-method changes touch accessibility (§16.3) and deserve the same care already given to keybinding presets.

Neither is built, spiked, or given a Tier 0 risk gate — flagged honestly rather than implied as done by virtue of appearing in this section.

---

## 68. Antigravity / VS Code Extension Manifest Import (amends §5, §67)

A later request asked for "Antigravity extension support." Antigravity's extension model is literally VS Code's — Antigravity is built on VS Code, and its extensions are VS Code extensions running in VS Code's actual extension host. Building real compatibility would mean building or vendoring exactly that host, which CLAUDE.md locks out in the flattest terms this document uses anywhere: *"Don't fork or vendor any VS Code/Monaco/CodeMirror code, ever, for any reason."* Rather than either quietly violate that or quietly ignore the request, this was raised explicitly and the scope below is the answer chosen: a manifest importer into Spartan's own WASM Plugin API (§5), not an extension host.

### 68.1 What Actually Converts

A VS Code/Antigravity extension's `package.json` declares its static contribution points under `contributes` — this is data, not code, and is the only part a manifest import can honestly touch:

- `commands` → registered as Spartan commands (`editor_api.commands.register`, §5.3), invokable from the command palette and bindable in Keybindings settings
- `keybindings` → imported as a starting keymap, editable afterward like any other binding (§16, Keybindings settings)
- `configuration` (the extension's declared settings schema) → rendered as real controls under Plugins & Extensions, using the same `SettingRow`/`Toggle` components every other settings category uses — not a raw JSON blob
- `snippets` and simple `grammars` (TextMate-style) references → straightforward data imports, no execution involved
- `themes` → already covered by §67.3's separate theme importer, cross-referenced rather than duplicated here

### 68.2 What Explicitly Does Not Convert, and Why That's Surfaced Rather Than Hidden

Anything past static contributions requires the extension's actual JavaScript/TypeScript `activate()` code to run against the real `vscode` API module (workspace, window, languages, debug namespaces) — that's the extension host, not a manifest, and there is no honest way to make it "just work" without building the thing CLAUDE.md prohibits. The importer's failure mode here matters as much as its success mode: an extension whose real functionality lives in its activation code (most language-feature extensions, most non-trivial extensions generally) imports its commands/keybindings/config as inert entries and reports plainly, per-capability, which contributions it could and couldn't bring over — never a silent partial success that looks more complete than it is. This is the same "don't fabricate, surface what didn't work" discipline §36/§48 already hold this project to, applied to a new feature instead of a bug report.

### 68.3 Where This Lives in the UI

Plugins & Extensions settings gains an "Import Antigravity/VS Code extension (.vsix)" row, the same dashed-border affordance pattern already used for MCP server and Skill imports — parses the manifest, shows the per-capability conversion report described in §68.2 before anything is installed, then installs the result as a normal, capability-scoped WASM plugin subject to the same permission prompt every other plugin gets (§5.2) — importing an extension is not a trust bypass.

### 68.4 Tier Placement

Tier 2 — sits behind Tier 1's MVP scope per §35, consistent with the Plugin Marketplace's own placement (§5.4, §35). Not urgent enough to pull ahead of Tier 1, but concrete enough now (conversion rules, UI location, explicit failure mode) to build directly from once its turn comes.

---

## 69. Spartan Mobile IDE (new product, design-only)

A later request asked to "begin creating the mobile version of the project, called Spartan Mobile IDE." Given the explicit choice to keep this spec-only for now — no branch, no repository, no code — this section is that design, written with the same rigor as every other subsystem in this document: what it actually is, what it deliberately is not, and how it relates to the desktop app it's a companion to.

### 69.1 What This Actually Is (and Is Not)

A phone screen is not a code-editing surface, and pretending otherwise would be dishonest about real mobile UX constraints rather than a design decision — no competitor's "mobile IDE" (GitHub Mobile, Linear's mobile app, even Antigravity itself, which has no mobile client at all as of this document's research pass, §67.1's sources) ships full text editing on a phone, and Spartan Mobile IDE does not either. It is a **companion app**, not a ported IDE: monitor and act on what Leo and the External Agent Fleet (§52) are doing, from a phone, when a desk isn't available.

**In scope for v1:**
- Inbox/Agent Manager mirror (§8, §50.1) — see running/review/done sessions across all workspaces, the same task-thread model as desktop, read from the same session store (not a separate mobile-only backend)
- Approve/Reject on Diff Cards and Implementation Plans (§8.5's Artifact model) — the single highest-value mobile action, since "someone needs to approve this before Leo can continue" is exactly the kind of interrupt-driven task a phone is good for
- Chat with Leo (text, tapping into the same conversation history as desktop/CLI, §46.2's "one Leo, two surfaces" principle extended to a third)
- Read-only, syntax-highlighted diff/file viewing for review context — not an editor, a viewer
- Push notifications for review-needed/CI-failure/mention events, reusing the existing Notifications settings categories and preferences (§42) rather than inventing mobile-specific ones
- Artifact commenting (§67's Antigravity-parity feature) — leaving feedback on a plan or diff from a phone is exactly the async-review use case this app exists for

**Explicitly out of scope for v1, not just unmentioned:** direct code editing, the GUI builder/Design View (§6), the debugger (§32), the Terminal panel (§59), and Developer Mode (§60) — a phone is the wrong form factor for path-jail-relaxed shell access, and this isn't offered rather than silently omitted.

### 69.2 Platform & Stack — A Different Call Than the Desktop App, Reasoned Through

The desktop app's from-scratch Rust+wgpu renderer (§2) is right-sized for a text-editing-and-rendering-performance problem the size of a full IDE. A companion app whose entire v1 surface is lists, diffs, chat, and push notifications does not have that problem, and building a second custom cross-platform mobile renderer to solve it would be over-engineering relative to what the app actually needs — the same "don't design for hypothetical future requirements" discipline this project's own contributor rules ask of any task. **Recommendation: a standard cross-platform mobile framework** (Flutter or React Native — a real choice between the two is deferred to whenever Tier assignment makes this actionable, not decided speculatively here) **rather than a custom native renderer**, talking to the same backend the desktop app and CLI already share (§46.2's session store, Project Graph, Artifact model) over the network rather than duplicating any of Leo's actual agent logic on-device.

### 69.3 Security Posture

Same account, same session store, same artifact trust model — not a separate, weaker auth path. Because there's no local shell/filesystem access at all in v1's scope (§69.1), most of §9/§36's hardening (path-jailing, Developer Mode, Untrusted-Repo Quarantine) simply doesn't apply to a client that never executes anything locally — this is stated as a consequence of the scope boundary in §69.1, not as an exemption carved out the way §60.2.1 carved out Developer Mode's one.

### 69.4 Relationship to §35's Roadmap and Tier Placement

This is a new product initiative, not an item inside the desktop app's existing Tier 0–3 roadmap (§35) — it has its own roadmap, starting from nothing, and does not borrow tier numbers from a different product's plan. Stated plainly: **nothing in this section is built, spiked, or scheduled** — no branch exists, no repository exists, this is the initial design pass only, by explicit choice when this was raised as an option rather than assumed.

### 69.5 Edge-Based Features — Revision Pass (amends §69.1–§69.3)

A later request asked to update this plan with more **edge-based** features — capability that lives on the device itself rather than assuming a network round-trip to the desktop/backend for everything. This matters more for this app than for most companions: the entire premise of §69.1 is "you're away from your desk," and away-from-desk is exactly where connectivity is least reliable. A companion app that goes blank in a subway, an airplane, or a job site with no signal fails at precisely the moment it exists for. Each feature below names its v1/v2 placement rather than dumping everything into an undifferentiated wishlist.

**Offline-first review queue (v1 — this revises §69.1's scope, not just extends it).** Pending artifacts (plans, diff cards, walkthroughs) sync to the device when connectivity exists and remain fully reviewable without it. Approve/Reject/comment actions taken offline are queued locally, signed at decision time, and replayed when the network returns — with one hard rule inherited from the Single Writer Invariant's spirit (§36.4.1): a queued approval is delivered as *"approved as of \<state the reviewer actually saw\>"*, and if the artifact changed while the decision was in flight, it surfaces as a conflict for re-review, never silently applied to a diff the reviewer never looked at.

**On-device model for local Q&A and summarization (v2).** A small quantized model (the same llama.cpp-family runtime the desktop's Ollama/LM Studio integrations already speak, §3.3/§57 — not a new inference stack) handles offline "explain this diff," "summarize what this session did," and artifact-text Q&A entirely on-device. Honest boundary, stated the way §66 states the CPU renderer's: an on-device 1–3B model is a *reading aid*, not Leo — it never plans, edits, or approves, and its answers are visibly badged as local-model output (§36.4.5's Model Integrity Guarantee applies on the phone too). PrivacyScoped routing (§3.5) extends here naturally: a repo marked local-only on desktop is local-only on mobile, so its artifact text never transits a cloud model from the phone either.

**Edge-cached repo context (v1, small).** The device keeps an encrypted local cache of recently-viewed diffs, files, and artifact history — scoped to what the user actually opened, not a full repo clone — so review context survives connectivity loss and reopening the app on a plane shows the same review you were reading at the gate.

**Biometric approval gating (v1).** The phone's one genuine security *advantage* over the desktop: destructive-action approvals (§36.4's approval matrix) and first-write-outside-project confirmations (§60.2.1) can require Face ID/fingerprint on mobile, binding the highest-stakes decisions to hardware-backed presence in a way a desktop keyboard can't. This is an *addition* to §69.3's posture, not a relaxation anywhere.

**Notification-surface actions (v1).** Approve/Reject directly from the push notification (with the artifact summary inline) for low-stakes artifacts; anything in the destructive class always requires opening the app and passing the biometric gate — the lock screen never approves a `git push --force`.

**Voice-to-task capture (v2).** On-device speech-to-text (the OS's own engine, not a cloud call) to dictate a new task into the Inbox — "Leo, look into why checkout tests got slow this week" — transcribed locally, submitted as an ordinary task thread when connectivity allows. This is deliberately narrower than §67.4's still-open "live voice transcription" desktop gap: capture-and-submit, not a live conversational voice mode.

**Camera capture into artifacts (v2).** Photograph a whiteboard sketch or an error message on another machine's screen; on-device OCR extracts text where legible, and the image attaches to a session as context the same way §14's log-line/stack-trace attachment already works. External Content Fetch Gating (§50.2) applies unchanged — a captured image containing a URL never triggers an automatic fetch.

**Network-aware sync policy (v1, small).** Artifact text/metadata sync freely on any connection; heavier payloads (screenshots, browser recordings, §65's trace assets) default to Wi-Fi-only with a per-item "fetch now anyway" override — a mobile-data bill is a real cost the same way §36.4.4 treats token spend as one, and it gets the same visible-before-incurred treatment.

**What stays out even in this pass:** on-device code *editing* or *execution* of any kind — the edge features above make reviewing, deciding, and capturing work offline; none of them move Leo's actual agent loop, the build system, or shell access onto the phone. §69.1's out-of-scope list survives this revision intact, and §69.3's security reasoning (no local execution surface) remains true of every feature added here except the on-device model, which executes inference only — sandboxed by the same posture §3.3 already applies to local runtimes on desktop, with no tool-calling surface at all on mobile.

### 69.6 Implementation Begun — the Two Deferred Calls Made, and Where the Code Lives

A later request asked to actually begin building this, reversing §69.4's "nothing in this section is built" by explicit choice — asked for and given the same kind of check-in §69.4 itself said this reversal would need. Two calls §69.2 had deliberately deferred are now made:

- **Framework: React Native**, not Flutter — §69.2 named both as real candidates and declined to pick speculatively; asked explicitly once "Tier assignment" (i.e., someone actually starting the build) made it actionable, and this was the answer given.
- **Where the code lives: a `mobile/` subdirectory of this repo, on this branch** — a later decision superseding this section's original call. It briefly lived in its own local repository (a separate-repo GitHub push wasn't reachable from this session — see the retired paragraph this replaces, still visible in git history) before an explicit instruction changed the call to a branch/subdirectory instead. Moved in via `git subtree`, preserving the mobile app's own real commit history intact as ancestors in this repo's graph rather than squashing it into one commit.

**What actually exists there right now** (verified by running it, not assumed, from its current location under `mobile/`): an Expo/React Native TypeScript scaffold with five real, type-checked, navigable screens — Inbox (session thread list, merging in locally-dictated tasks), SessionDetail (chat + pending-artifact link + camera capture into attachments), ArtifactReview (read-only diff viewer, biometric-gated Approve/Reject, artifact commenting, an on-device-model Q&A affordance), Settings (notification permission, connectivity + offline-queue status, notification-preview triggers), and NewTask (voice dictation) — wired through real React Navigation, backed entirely by mock data since no session-store backend exists yet. §69.1's full v1 list and §69.5's full v1+v2 list are now built, at three explicitly different confidence levels rather than one flattened claim: Expo-Go-compatible modules (notifications, biometrics, secure storage, async storage, network info, image picker) have real reason to work on a real device; `expo-speech-recognition` (voice-to-task capture) is a third-party native module needing a custom dev client to actually run, so its clean bundle proves only that its JS/TS glue resolves; on-device model Q&A stayed a deliberate, honestly-labeled stub (`mobile/src/lib/localModel.ts`) rather than a faked integration, since a real one needs a llama.cpp-family binding, a multi-GB model file, and inference hardware, none of which exist in this environment. Confirmed clean throughout, both before and after the move to `mobile/`: `npx tsc --noEmit` and `npx expo export --platform android` (a real 928-module Metro bundle, identical bundle hash before and after relocating, confirming the move changed nothing about the app itself). Confirmed **not** done: this has never run on an actual device, emulator, or simulator — none was reachable in this environment, the same no-GPU/no-display constraint §47.3 already documents for the desktop app. Also not built: a real push/sync backend (every "queued"/"cached"/"local only" note in the mobile code means exactly that), on-device OCR for captured images, and an actual model behind the Q&A stub — see `mobile/README.md` for the same status kept in sync with this paragraph.

### 69.7 Feature Enhancement Pass — Closing a Spec/Implementation Gap, Deepening Three Screens, a New Decision History Feature, and Full Screen-Level Test Coverage

A later request asked to both add genuinely new mobile-only features and deepen what already existed, rather than treating §69.6's "already built" as a stopping point. Run for real against the code at its current location under `mobile/`, not designed in the abstract:

**A real gap between this spec and the implementation, closed.** §69.1 promised "read-only, syntax-highlighted diff/file viewing," but `ArtifactReviewScreen` was rendering unified-diff patch text as a single flat, uncolored `<Text>` block — a claim in this document that the running code didn't actually satisfy. `mobile/src/lib/diffHighlight.ts` now parses a patch into typed lines (`add` / `remove` / `hunk` / `context` by leading `+`/`-`/`@@` marker — deliberately minimal, matching exactly what this app's real patches look like, not a general-purpose diff-parsing library) and the screen renders each line colored accordingly. This is a correction to bring the app in line with what §69.1 already said, not new scope.

**Two deepened screens, still within §69.1/§69.5's documented boundaries.** `SessionDetailScreen`'s "Chat with Leo" had no way to actually send a message — only a read-only log of pre-seeded mock messages — so it wasn't really a chat. It now has a message composer (local `useState`, the same pattern `ArtifactReviewScreen`'s comment composer already used, so no new store module was introduced for something that doesn't need cross-restart persistence any more than comments do); sending a message appends the user's own bubble only — no fake Leo reply or typing indicator, since inventing one would misrepresent a backend response that doesn't exist. Separately, `InboxScreen`'s single flat, unfilterable thread list gained a search box (title/workspace substring match) and a status filter (All/Running/Review/Done, reusing `StatusPill`'s existing color palette) — basic list usability, not new product scope.

**Per-file commenting, completing a data model that was already there.** `ArtifactComment.filePath: string | null` existed from the start, but the UI never actually set it to anything but `null`. `ArtifactReviewScreen` now has a "Comment on this file" affordance per file block; posting a comment while one is targeted carries the real path instead of always `null`.

**A genuinely new feature, not previously in this section: Decision History (v1).** A local, persistent log of every Approve/Reject decision made on the device, across both places a decision can actually happen — `ArtifactReviewScreen`'s biometric-gated review flow and `notificationActions.ts`'s direct low-stakes notification-button flow. Both already funneled through `decisionActions.ts`'s `recordDecision` as a single choke point; `mobile/src/lib/decisionHistory.ts` (mirroring `offlineQueue.ts`'s exact AsyncStorage read/write/defensive-parse pattern) is now called from that one choke point for every branch that actually records a decision (not the denied branch, where nothing was recorded), so no call site had to duplicate the bookkeeping. `DecisionHistoryScreen` (reachable from a new row in Settings) lists entries newest-first with a "queued — not yet synced" indicator and a "Clear history" action. Honest boundary, same as everywhere else in this section: this is a local audit log, not a synced one — there is still no backend to reconcile it against, and nothing here implies one exists.

**Full screen-level component test coverage, not just the business-logic layer.** The mobile app's first Jest test pass covered `src/lib`/`src/data` only, explicitly leaving "the screens themselves as rendered React components" as a documented gap. That gap is now closed: all six screens (the original five plus `DecisionHistoryScreen`) have real `@testing-library/react-native` component tests — 93 tests total across 16 suites, run for real with `npx jest --ci`, not merely written. Every test suite was spot-checked by deliberately breaking the real behavior it claims to cover and confirming the test actually failed before restoring the code, the same discipline already established for the `src/lib` test pass. One real, non-obvious environment finding surfaced only by running these, not by reading library docs: this repo's installed `@testing-library/react-native` 14.x (paired with React 19's concurrent renderer) makes `render()` and `fireEvent.*` calls asynchronous — they must be `await`ed, or queries silently fail or events never flush before assertions run. This is a dependency-version fact worth other implementers knowing before they hit the same failure mode blind.

Verified end to end after all of the above: `npx tsc --noEmit` clean, `npx jest --ci` — 93/93 passing across 16 suites, `npx expo export --platform android` — a real 931-module Metro bundle, succeeds. Still not run on an actual device, emulator, or simulator, for the same reason §69.6 already gives: none is reachable in this environment.

### 69.8 Visual Theme Parity with Desktop (amends §50.3)

A later request asked for the mobile app to use "the same theme as the desktop IDE." Until this pass, mobile had never adopted §50.3's high-contrast, Antigravity-researched palette at all — every screen used its own ad hoc light-theme colors (white backgrounds, a generic Tailwind blue `#2563eb` for primary actions), unrelated to the desktop reference prototype's actual tokens. Fixed by importing the exact same values rather than approximating them fresh a second time: `mobile/src/theme.ts` now re-declares `interface-prototype.jsx`'s `C` token object verbatim (`bg #09090B`, `s1/s2/s3` surfaces, `border`/`borderLt`, `text`/`textMid`/`textDim`, the `accent` rust/terracotta hue `#C4432B`, `green`/`amber`/`red`/`teal` and their `*Bg` variants) and a `STATUS_COLOR` map matching `STATUS_META` exactly (`running → accent`, `review → amber`, `done → green`; desktop's fourth state, `paused`, has no mobile equivalent since `SessionStatus` doesn't include it, and wasn't invented to fill the gap).

Applied everywhere a color was previously hardcoded: all six screens, `StatusPill`, and a new `navigationTheme` (React Navigation `Theme` object) plus `screenOptions` in `RootNavigator` so header bars and screen backgrounds come from the same tokens by default instead of each screen re-deriving them. `App.tsx` now renders `<StatusBar style="light" />` and `app.json`'s `userInterfaceStyle` changed from `"light"` to `"dark"`, since the app is now unconditionally dark-themed rather than following the OS light/dark setting. One color has no desktop equivalent and was deliberately reassigned rather than invented: the on-device-model button was an off-palette purple (`#7c3aed`) with no counterpart in `C`; it now uses `teal`, an existing token otherwise unused for primary actions on mobile, keeping it visually distinct from Approve/Reject/Send without introducing a color outside the shared palette.

One correctness gap the retheme surfaced and fixed along the way: several `Text` styles (`InboxScreen`'s row title, `ArtifactReviewScreen`'s file path and loading/not-found states, `SettingsScreen`'s label/section headers, `DecisionHistoryScreen`'s entry title) had no explicit `color` at all — they relied on the platform's default near-black text color being legible against the *old* white background. That assumption silently breaks on a dark background (near-black text on near-black background), so every such style now sets `color: C.text` explicitly rather than depending on a default that was never actually theme-aware.

**Deliberately not attempted in this pass**: desktop's typography tokens (`F_UI`: Space Grotesk, `F_MONO`: IBM Plex Mono) were not ported to mobile, which keeps system fonts. Matching them would need `expo-font` plus the corresponding `@expo-google-fonts` packages, async font loading before first render, and a loading-state decision — a real, separable follow-up, not a silent scope-narrowing of "same theme."

Verified end to end: `npx tsc --noEmit` clean, `npx jest --ci` — 93/93 passing across 16 suites (one test file's own `@react-navigation/native` mock had to be fixed to spread the real module rather than replace it outright, a pre-existing over-narrow mock that only surfaced once `theme.ts` added a real dependency on that module's `DarkTheme` export), `npx expo export --platform android` succeeds. Confirmed visually via a real running instance (Expo web / `react-native-web`, not a native device/emulator — none reachable in this environment) across all six screens, including a live end-to-end check that an actual recorded decision still renders correctly in the retheme Decision History screen.

---

## 70. Import & Migration — Projects and Preferences from Other AI Tools (amends §42, §63, §64, §16)

A later request asked for settings to import both **projects** and **user preferences** from other AI coding tools. This section names the concrete mechanism rather than leaving "import from other AI" as a vague catch-all — same discipline §67/§68 already applied to Antigravity specifically, generalized to the wider set of tools §36.1 already treats as Spartan's competitive set (Cursor, Windsurf) plus the broader roster the External Agent Fleet (§52) already knows how to detect (Claude Code, Aider, Cline, Continue, GitHub Copilot).

### 70.1 What "Projects" Means: Per-Workspace Config Detection

Opening a project scans for marker files other tools use for project-level instructions and config — the same auto-detection posture §20.1 already uses for language toolchains, applied to AI-tool config instead:

| Tool | Recognized markers |
|---|---|
| Cursor | `.cursorrules`, `.cursor/rules/*.md`, `.cursor/mcp.json` |
| Windsurf | `.windsurfrules`, `.windsurf/` |
| GitHub Copilot | `.github/copilot-instructions.md` |
| Cline | `.clinerules` |
| Continue | `.continue/config.json` |
| Aider | `.aider.conf.yml`, `CONVENTIONS.md` |
| Claude Code | `CLAUDE.md`, `.claude/settings.json` |

A detected marker surfaces as a real, actionable row in the new **Import & Migration** settings category (§42.2) — never a silent auto-import on project open, consistent with §36's standing rule that nothing configuration-relevant changes without the user seeing it happen.

### 70.2 What "User Preferences" Means: Global Settings Import

Distinct from per-project detection: a one-time import from an exported/connected account or config directory, mapped item-by-item onto Spartan's own settings:

- Custom instructions / rules files → imported as a Project- or Global-scoped **Skill** (§63), not a special-cased second instructions system — a `.cursorrules` file and a hand-written Skill are the same underlying concept, so they get the same home
- MCP server definitions → imported as **MCP Server** entries (§64). This is the one category with genuinely high-fidelity conversion: the MCP protocol's config shape (command, args, transport) is the same JSON structure across Cursor, Windsurf, Claude Desktop, and Spartan itself, so this import is closer to a format translation than an interpretation — worth calling out as qualitatively more reliable than the rest of this section, not lumped in with the lossy cases
- Keybinding preset → mapped onto Keybindings settings' existing preset picker (§16, §42.2) if it matches a known preset family, otherwise imported as a new custom keymap
- Theme → routed through the same VS Code theme importer already built (§67.3) rather than a second theme-conversion path
- Model/provider preference → mapped onto Leo & Models (§3, §44) where an equivalent provider exists (e.g., "prefers Claude Sonnet" carries over cleanly); explicitly reported as not applicable where the source tool's preferred model has no Spartan equivalent, rather than silently dropped or approximated

### 70.3 What Never Silently Imports: Approval/Autonomy Posture

The one deliberate exception, named rather than left implicit: an imported tool's auto-approval or "YOLO mode"-equivalent setting is never applied to Spartan's own autonomy level (§4.1, §42.2) automatically, even if every other preference in the same import batch applies cleanly. It's surfaced as its own separate, explicitly-labeled item in the conversion report, requiring the same one-time confirmation §60's Developer Mode widening already requires for its own scope change — importing a more permissive posture from somewhere else is exactly the kind of security-relevant change §36.4's hardening exists to gate, and "the user imported it from another tool" is not an exemption from that gate.

### 70.4 The Conversion Report, Same Discipline as §68.2

Every import — project-level or preference-level — produces the same per-item report already established for extension import (§68.2): each thing attempted is marked converted or not, with a stated reason for anything that didn't, never a silent partial success. Built into the reference prototype's new Import & Migration settings category rather than only described here.

### 70.5 Tier Placement

Tier 1 for the MCP-server and theme import paths (genuinely high-fidelity, low-risk, and cheap given both underlying importers already exist per §64/§67.3); Tier 2 for instructions/rules-file-to-Skill conversion and the full per-project detection UI, since interpreting a freeform rules file well enough to be a good Skill is a harder, lower-urgency problem than the mechanical conversions.

---

## 71. Leo Chat — Antigravity 2.0 Parity Pass (amends §8, §50.1, §67)

A later request asked for Leo's chat to look and function like Antigravity 2.0. Researched fresh rather than restyled from assumption, same discipline as every other Antigravity-fidelity pass in this document (§50, §67): Antigravity's own documentation and independent write-ups describe its chat as, in their own words, "a chatbot interface like every other AI UI" for the side panel, with the more distinctive design choice being *what it deliberately doesn't show* — rather than raw tool-call output, agents produce **Artifacts** (implementation-plan/task/walkthrough markdown files, diffs, screenshots, browser recordings), because scrolling through raw tool calls is tedious. Spartan's Auxiliary Pane (§8.5) already works this way — Implementation Plan, Task List, and Diff Cards artifacts instead of a raw tool-call log — so the side chat itself needed confirmation, not a redesign: this was already substantively matched before this pass, not something to rebuild from scratch.

### 71.1 What Was Actually Missing: A Second Chat Surface

The one real, distinct gap: Antigravity has **two** separate chat surfaces, not one — a side panel "for conversations and broad changes," and **inline chat** (`Cmd+I`/`Ctrl+I`) that "operates on the exact code you're looking at," described as the scalpel to the side panel's broader reach. Spartan's reference prototype only had the side panel (`AgentView`) before this pass.

**Built for real, not just described**: `EditorView` now has a genuine `Cmd+I`/`Ctrl+I` keydown handler (real `window.addEventListener`, not a mocked button) that opens a small chat box scoped to whichever code line is currently selected — `Esc` closes it, clicking a different line re-scopes it. This is a second, narrower surface, not a replacement for the side chat — both stay, matching Antigravity's actual two-surface design rather than collapsing them into one.

### 71.2 Walkthrough — the Third Artifact Type

Antigravity's per-conversation storage holds three core markdown documents: `implementation_plan.md`, `task.md`, and `walkthrough.md`. Spartan already had the equivalents of the first two (Implementation Plan, Task List artifacts) but nothing for the third — a written recap of what was actually done, produced after the fact rather than tracked live like the task list. Added as a fourth `ArtifactShell` in the Auxiliary Pane, populated with real per-session recap text in the reference prototype rather than a placeholder.

### 71.3 What This Pass Deliberately Left Alone

The side chat's visual styling (message bubbles, model badge, input box) was not overhauled, because the research this pass is built on describes it as an ordinary chat UI, not a distinctive one — and §50.3's high-contrast palette pass already brought Spartan's tokens close to Antigravity's own. Redesigning a component that both research and a prior verified pass already say is a good match would be busywork dressed up as a feature, not a real gap closed. Consistent with §36.4.10: this is additive to the chat surface, not a replacement of it — the side chat, its artifacts, and its message history all still work exactly as before.

### 71.4 Tier Placement

Tier 1 — inline chat and the Walkthrough artifact are both natural extensions of already-Tier-1 surfaces (the editor, the Auxiliary Pane) rather than new subsystems, and the "second chat surface" gap was concrete and cheap to close once identified.

---

## 72. IoT & Embedded Development Support (amends §20, §21, §32, §33)

Android (§21) got a dedicated first-class subsystem rather than being treated as "just another `LanguageProfile`," because device management, flashing, and on-device debugging need more than an LSP/DAP command pair. IoT and embedded targets need the same treatment, for the same reason — this section is that treatment, not a bullet added to §20's language table.

### 72.1 Board & Toolchain Registry

```rust
struct IoTBoardProfile {
    id: BoardId,                  // "esp32", "arduino-uno", "rpi-pico", "nrf52840", "stm32f4", ...
    toolchain: IoTToolchain,       // PlatformIO | ArduinoCli | EspIdf | ZephyrSdk | MbedCli
    flash_command: CommandSpec,
    serial_baud_default: u32,
    rtos: Option<RtosKind>,        // FreeRTOS | Zephyr | RIOT | None (bare-metal)
}
```

- Curated default registry covering the boards that actually dominate hobbyist and production IoT work: Espressif ESP32/ESP8266, Arduino Uno/Nano/Mega and ESP32-based Arduino boards, Raspberry Pi Pico (RP2040), STM32 Nucleo/Discovery boards, Nordic nRF52/nRF53, Particle Photon/Boron
- **PlatformIO as the default toolchain layer** wherever a board supports it — it already normalizes 1000+ boards and multiple frameworks (Arduino, ESP-IDF, Zephyr, mbed) behind one build/upload/monitor CLI, so Spartan wraps it the same way §32 wraps LLDB/Delve rather than reinventing a device database from scratch. Arduino CLI, ESP-IDF, Zephyr SDK (`west`), and mbed CLI remain available directly for boards or workflows PlatformIO doesn't cover well
- Auto-detection via serial-port enumeration — the same USB-device-scanning primitive the Devices panel (§33) already uses for Android, generalized rather than duplicated, not a second device-discovery system living side-by-side with the first

### 72.2 Serial Monitor

A new tab in the existing Devices panel (§33) rather than a bespoke standalone panel — real-time UART/serial output, timestamped, filterable by log level where the firmware emits one, with a send-line input for interactive REPL boards (MicroPython, CircuitPython). Reuses the dock's existing hide/show pattern (§62.2) like every other panel.

### 72.3 Flash & OTA Workflow

- Flashing is a `Task` (§20.2) like any build step — "Build" → "Flash" as a dependent task pair, with progress streamed the same way any other Task's output streams, not a special-cased progress bar
- **OTA update support** where the toolchain provides it (ESP-IDF's OTA partitions, Particle's cloud-flash) is a distinct Task variant, explicitly labeled as network-capable and subject to the same approval posture as any other network-capable tool call (§9) — pushing new firmware to a device over the network is not the same risk class as flashing over USB, and the UI doesn't pretend otherwise

### 72.4 Protocol-Level Debugging

- **MQTT Inspector**: a subscribe/publish client with a live topic tree, reusing the live-traffic-panel pattern already built for the Playwright browser panel (§65) rather than requiring a separate tool like MQTT Explorer
- **RTOS-aware debugging** extends §32.2's OpenOCD+GDB entry: where the target runs FreeRTOS or Zephyr, the debugger's thread list shows RTOS tasks by name (OpenOCD's built-in `rtos` config support), not raw stack pointers — an embedded stack trace without RTOS awareness is close to unreadable, so this isn't a cosmetic nicety
- **BLE/Zigbee sniffing** via existing platform tooling (Nordic's nRF Sniffer, Wireshark's BLE/Zigbee dissectors) surfaced as a Spartan panel instead of requiring an app switch

### 72.5 Security Note — Direct Tie to §73

IoT is one of the most consistently under-secured categories of shipped software (the Mirai botnet and its many descendants are the canonical case, not an edge case) — default credentials, unencrypted device-to-cloud traffic, and debug interfaces left enabled in production builds are common failure patterns. §73's Exploit Auditor names IoT-specific checks explicitly for exactly this reason, rather than assuming generic web/API security checks already cover an embedded target's actual attack surface.

### 72.6 Tier Placement

Tier 2 — a platform investment in the same size class as Android's (§21), sequenced behind Tier 0/1's remaining core-engine and MVP work per §35, not pulled ahead of it because embedded development is a compelling area.

---

## 73. Security & Exploit Auditor — Verified Findings, Not Just Static Warnings (amends §27, §9, §36)

A later request asked for "a security and exploit auditor for created projects" — auditing and, where safe, verifying the exploitability of vulnerabilities in the user's own project. This is squarely defensive, authorized security work: testing your own application's security posture is standard practice, the same category as running OWASP ZAP or Burp Suite Community against your own lab instance, and this section is designed the same way every dual-use-adjacent capability in this document already is — scoped tightly, gated by explicit approval, and never capable of being pointed at anything but the project you're actively working in.

### 73.1 Why This Goes Beyond §27's SAST/DAST Panel

Static analysis is notorious for false-positive fatigue — flagging a theoretically-vulnerable pattern without confirming it's actually reachable in the real running app, which trains users to ignore the scanner. The Exploit Auditor's job is narrower and more valuable: take a subset of §27's static findings and **actively verify** exploitability against a locally-running instance of the user's own project, then produce a Verification-style Artifact (§8.5) — the vulnerability class, the exact reachable input/path that triggers it, a sandboxed proof-of-concept demonstrating impact without exfiltrating real data, and a suggested fix Leo can propose as a normal reviewable diff.

### 73.2 Scope — a Hard Boundary, Not a Suggestion

- **Only ever targets** code in the currently open, user-owned project, running on localhost or an explicitly-configured local/staging environment the user has designated for this purpose — never a URL or host outside the project's own configured dev environment. This is an allowlist enforced structurally, the same posture path-jailing (§36.4.6) already takes toward the filesystem, not a warning dialog someone can click through.
- **Every active-verification run requires its own explicit, per-run approval** — never runs automatically or silently, even under Autonomous or Vibe Mode autonomy settings (§45), the same category of standing exception Untrusted-Repo Quarantine (§36.4.2) already carves out of the normal autonomy model for a different risk.
- The tool has no code path that accepts an arbitrary external host as a target — this is a structural refusal, not a policy the UI merely discourages.

### 73.3 What It Actually Audits

- **OWASP Top 10-style web vulnerability classes** for web targets (SQL injection, XSS, SSRF, auth bypass, IDOR, insecure deserialization) — verified via the same Playwright-driven live-browser panel already built for testing (§65) rather than a second, duplicate browser-automation stack: Leo drives the local instance, observes actual behavior, and confirms exploitability instead of guessing from source alone
- **Reachability-checked dependency CVEs**, extending §27's CVE feed: a flagged CVE is cross-referenced against the Project Graph (§30) to confirm the vulnerable code path is actually reachable from this project's own call graph, not just "this package version has a CVE" regardless of whether the vulnerable function is ever called
- **Secrets exposure verification**: confirms a flagged hardcoded secret (§9's secrets-scanning pass) is actually loaded and used at runtime before treating it as a live-severity finding, rather than flagging dead code at the same severity as a real exposure
- **IoT/embedded-specific checks** (§72.5): default credentials left in firmware, unencrypted MQTT/CoAP traffic, missing TLS on device-to-cloud communication, debug/JTAG interfaces left enabled in a release build
- **Infra-as-code misconfiguration**, tying into the Ops View (§23): overly permissive IAM policies, open security groups, unencrypted storage buckets, surfaced through the same IaC diff scanning §23 already does for deploys

### 73.4 Findings Are Artifacts — Same Discipline as Everywhere Else

A confirmed-exploitable finding becomes a Verification-style Artifact (§8.5): CVSS-style severity, the verified reproduction steps, and a proposed fix Leo can turn into a normal reviewable diff — **never auto-applied**, same Accept/Reject Diff Card flow as any other change (§8.5). An unverified, static-only finding from §27's existing pass is visibly labeled as such — "flagged, not yet verified exploitable" — so the two confidence levels are never conflated in the UI, the same "never imply more certainty than actually exists" discipline this document holds itself to about its own build status (§47, §51).

### 73.5 Tier Placement

Tier 2 — §27's static SAST/DAST/secrets/SBOM pass is more foundational, lower-risk, and appropriately Tier 1-adjacent; the active-verification layer this section adds is a deeper investment correctly sequenced after it, not built first because "exploit" sounds more interesting than "static scan."

---

## 74. Open Source Decompiler Integration (amends §73, §32, §21, §20)

A later request asked to add open source decompilers — reverse-engineering a compiled binary back into readable pseudocode/disassembly. This is standard, legitimate tooling for security research, malware triage in a defensive context, license-compliance verification, and understanding a third-party dependency's actual behavior before trusting it — the same authorized, defensive posture §73's Exploit Auditor was built under, and this section reuses that trust model rather than inventing a second one.

### 74.1 Why This Belongs Next to the Exploit Auditor

Decompilation is a core primitive underneath legitimate security research: analyzing a vendor binary for vulnerabilities before deploying it, triaging a suspicious sample defensively, or verifying a third-party `.so`/`.dll`/APK doesn't do something undocumented. §73 already established the disciplined pattern this needs — scoped, gated, never silently escalating trust — so §74.7 reuses it explicitly instead of drafting a parallel policy.

### 74.2 Decompiler Registry

Same registry pattern as `LanguageProfile`/`DebugAdapterProfile`/`IoTBoardProfile`:

```rust
struct DecompilerProfile {
    id: DecompilerId,                 // "ghidra", "radare2", "jadx", "ilspy", "cfr", ...
    input_formats: Vec<BinaryFormat>, // ELF, PE, Mach-O, APK/DEX, .NET assembly, JVM class, wasm
    engine: DecompilerEngine,
    headless_command: CommandSpec,    // scriptable, so Leo can drive it as a normal tool call
}
```

### 74.3 Primary Engine: Ghidra

**Ghidra** (NSA, Apache 2.0, genuinely the most capable open source decompiler by architecture coverage — x86/x86-64, ARM/AArch64, MIPS, PowerPC, RISC-V, and more) is the default, wrapped via its headless analyzer (`analyzeHeadless`) with a scripted bridge so Leo can drive it as a tool call, not only a GUI a human operates by hand. Decompiled pseudocode surfaces in a real read-only editor view — reusing the existing rope/renderer/tree-sitter pipeline (§2) rather than a bolted-on second viewer — with cross-reference navigation (jump to caller/callee) backed by the Project Graph (§30) the same way source-symbol navigation already works.

### 74.4 Fast-Triage Engine: radare2 / Cutter

**radare2** is wrapped for quick, scriptable triage — disassembly, string extraction, function identification — with a faster startup than Ghidra's full analysis pass, useful as a first look before committing to a full decompile. Cutter's control-flow-graph view is surfaced as a panel reusing Design View's existing canvas rendering (§6) rather than a second graph-rendering system built from scratch.

### 74.5 Platform/Language-Specific Decompilers

| Decompiler | Targets | Ties to |
|---|---|---|
| **JADX** | Android APK/DEX | §21's first-class Android support — reviewing a third-party APK's actual behavior before trusting it, or recovering readable Kotlin/Java-shaped code from an app's own build artifact |
| **CFR** / **Fernflower** | JVM `.class`/`.jar` | §20's existing Java/Kotlin/Groovy profiles — Fernflower is the same engine IntelliJ itself embeds |
| **ILSpy** | .NET/CIL assemblies | §20's C#/F# profiles, the same CLR family `netcoredbg` already debugs (§32.3) |
| **uncompyle6** / **decompyle3** | Python bytecode | An honest niche case — source-lost recovery, not oversold as commonly needed |
| **wasm-decompile** (WABT toolkit) | WebAssembly | §20.1.2's Emscripten/`wasm-pack` compilation targets — completes the round trip: compile to `.wasm`, decompile one you didn't build yourself back to readable form |

### 74.6 Where This Lives in the UI

A **Decompiler panel** — same dock-panel family as Terminal/Playwright/Devices, same hide/show pattern (§62.2) — pick a binary, pick an engine (or let Spartan auto-select from the detected format), get a read-only pseudocode/disassembly view with the same search/navigation as any source file. Decompiled output is never written back into the original binary; it's read-only, source-adjacent material Leo can read and reason about ("explain what this function does," §13.4's explainability tools), not something edited and reassembled.

### 74.7 Security Posture — Explicit Reuse of §73's Scope Discipline

Analyzing a binary you don't control the source of is different risk territory than analyzing your own project's source — a decompiled third-party binary is untrusted content by construction. Rather than invent a new policy, this reuses **Untrusted-Repo Quarantine** (§36.4.2): any binary opened for decompilation that isn't a build artifact of the currently-open, already-trusted project is treated as untrusted content — no auto-run, no auto-execution of anything extracted from it. Strings/resources pulled from a decompiled sample go through the same secrets-scanning pass (§9) before being rendered verbatim, and any URL found inside decompiled output is subject to **External Content Fetch Gating** (§50.2) exactly like one found in repo content — a decompiled malware sample's embedded strings are precisely the content class that gating exists for. Leo handing a decompiled function to itself as vulnerability-triage context (feeding §73's auditor) uses the same tool-context approval posture as any other handoff — this is a read/analyze capability, never an autorun one.

### 74.8 Tier Placement

Tier 2 — the same class as the Security Auditor (§73) it's built to complement. Ghidra's headless integration is the concrete, valuable core; the platform-specific decompilers (JADX/CFR/ILSpy/etc.) layer in afterward as individually smaller, more contained wins once the registry/panel pattern already exists.

---

## 75. Tier 1 Implementation — Real Build Begun (Core Buffer + Language Registry)

A later request asked to actually build the IDE from this spec, rather than keep it at the design-and-spike stage. "Create the IDE from spec" taken literally is not something one pass can honestly deliver: even Tier 1 alone (§35.4, "the minimum one-of-a-kind IDE") spans a custom renderer, a full agentic core, 5–6 language profiles, Android support, and more — realistically months of engineering — and this environment has no GPU or display, confirmed repeatedly since §47.3. Rather than fake completeness or silently pick an arbitrary slice, the scope was asked about directly; the choice made was to start with the two pieces of Tier 1's core-engine/language-support work that are both genuinely required by §35.4 and fully testable without a display: the real document/buffer model (§2.1) and the `LanguageProfile` registry (§20.1).

### 75.1 What Was Actually Built

- **`crates/spartan-buffer`** — the real `Document` type from §2.1: `ropey`-backed, char-indexed insert/delete/replace, a branching undo/redo checkpoint tree (not a linear stack — `jump_to_checkpoint` reaches any prior checkpoint directly, the actual "point at an old root" branching behavior §2.1 calls for, proven by a test that edits down one branch, undoes, edits down a *different* branch, then jumps back to the first branch's checkpoint directly), and a bounded ring of the last N checkpoints (default 500, matching §2.1's own default) that evicts purely by creation order. Periodic full checkpoints to disk (the other half of §2.1's crash-recovery design) are not yet implemented — named as a real gap, not silently assumed done.
- **`crates/spartan-languages`** — the real `LanguageRegistry` from §20.1: the `LanguageProfile` struct exactly as specified (plus a `marker_files` field the spec's prose implies but never actually added to the struct — added here since detection needs it as data, not a hardcoded `match`), a curated `languages.toml` seeded with precisely the Tier 1 six languages §35.4 names (Rust, TypeScript, Python, Kotlin, Java, Go) rather than the full ~40, extension-glob file matching, and marker-file project detection that correctly returns multiple profiles for a genuinely polyglot repo — exercised by a test using §20.2's own example (a Rust core + Kotlin app + TypeScript frontend in one repo).

Both are real crates in the Cargo workspace, under `crates/` — deliberately a different top-level directory from `spikes/`, since these are product code being built toward Tier 1, not Tier 0 risk-gate experiments with a go/no-go verdict. 15 and 10 tests respectively (45 total across the whole workspace including the four spikes), `cargo clippy --workspace --all-targets --release` clean, `cargo fmt --check` clean.

### 75.2 Real Bugs Found and Fixed, Not Assumed Away

Consistent with this document's own §48/§51 discipline — two genuine bugs were caught only by actually running the tests:

- **The first eviction design was wrong.** An early version of the ring-buffer eviction protected every ancestor of the current checkpoint from being dropped — the intuitively "safe" choice. But for an ordinary, unbranched typing session (the common case, not an edge case), *every* checkpoint is an ancestor of current, so nothing was ever evicted and the "last N edits" ring never actually bounded memory in exactly the case that matters most. Caught by a test's `checkpoint_count()` assertion actually failing, not by re-reading the code and deciding it looked fine.
- **A real panic in `undo()`.** Fixing the eviction bug above exposed a second, distinct issue: `undo()` indexed the parent checkpoint directly (`self.checkpoints[&parent]`), which panics once that parent has legitimately aged out of the ring under the corrected eviction logic. Fixed to check first and treat an aged-out parent as "nothing further to undo" — a normal `false` return — rather than a crash. Found by a test written specifically to exercise the boundary (`undo_stops_cleanly_once_history_ages_out_of_the_ring`), not by inspection.

### 75.3 What This Still Does Not Mean

This is the start of Tier 1's core-buffer and language-registry pieces — not the IDE, and not claimed as such. Not yet done, named plainly rather than left implicit: GPU rendering, wiring the registry's `lsp_command`/`dap_command` fields into the already-proven `lsp-spike`/`dap-spike` clients (a natural next step, genuinely smaller than what was just built, but not done in this pass), tree-sitter integration, Leo's agentic loop operating on these buffers, and any UI at all. The GPU-rendering and custom-UI-skeleton Tier 0 spikes (§35.3) remain unrun in this environment for the same reason as every prior pass — no display/GPU reachable here (§47.3) — and this pass didn't attempt to route around that constraint.

### 75.4 Frozen Reference

A point-in-time copy of the spec exactly as it stood immediately before this implementation work began is preserved at `docs/architecture-spec.SNAPSHOT-2026-07-04-pre-implementation.md` — untouched as the living spec (this file) keeps evolving, so "did the build match the spec it started from" has a stable answer.

### 75.5 Real Increment: `crates/spartan-editor-core` — Buffer + Renderer + Language Registry Combined, Viewport Virtualization

§75.3 named GPU rendering, the registry's LSP/DAP wiring, and any UI at all as the next real gaps. Separately, `spikes/render-spike` (Tier 0, §47.9-§47.10) proved a real wgpu/glyphon rendering approach and, via its damage-region CPU-shaping increment, got per-edit latency close to §39.1's target — but its own exit report named cold-open (~900-1300ms vs. a <100ms target) as untouched, because its `TextState` always received the *entire* document via `Buffer::set_text` regardless of viewport size, and it never scrolled at all. This increment is a real (non-spike) `crates/spartan-editor-core` that combines `spartan-buffer`, a promoted-and-improved copy of render-spike's rendering approach, and `spartan-languages` for the first time in one real file-open — plus one genuinely new piece of engineering: **viewport virtualization**, the literal reading of §2.2's damage-region requirement ("only re-rasterize the visible viewport + scroll buffer").

**What was built.** A `Viewport { scroll_line, visible_lines }` struct and a `windowed_text(&Document, &Viewport)` function that extracts only the currently-visible slice of the document — cosmic-text's `Buffer` now only ever sees ~34-60 lines, never the full file, regardless of document size. `EditEffect::Line(doc_line_i)` edits are translated to a window-local index via `to_local_line`, which returns `None` (no redraw at all) if the edit happened off-screen — a real virtualization win render-spike never had the opportunity to take, since it always rendered the whole document. Scrolling (new — render-spike never scrolled) re-slices and fully re-shapes the window. `language::detect_language_for_file` wires in the real `spartan-languages` registry on file open, printing the detected profile's id, tree-sitter grammar, and LSP/DAP availability — tree-sitter and LSP/DAP themselves stay unwired, only the lookup is real. GpuState, CursorRenderer, LatencyTracker, and the `EditEffect` classification were promoted from render-spike essentially verbatim.

**Real measured results**, run on the same Intel UHD Graphics 620 / Vulkan / IntegratedGpu hardware as render-spike, against the identical 50,000-line synthetic fixture, reported directly alongside render-spike's own post-damage-region numbers rather than in isolation:

| Metric | render-spike (post damage-region) | spartan-editor-core (+ virtualization) | Verdict |
|---|---|---|---|
| Cold-open | 897.7-1297.9ms | 575.5-617.5ms (3 runs) | ~1.6-2.2x faster; still ~6x over the <100ms target, not closed |
| Edit p99, random-position across whole document | 6.0-25.1ms | 2.5-3.1ms | Faster, but see caveat below |
| Edit p99, realistic (cursor-adjacent) typing | 6.0-25.1ms (render-spike had no other kind) | 3.5-3.9ms (2 runs, n=500 each) | Meaningfully better, and now reliably under the <5ms p99 target |
| Scroll re-shape | not measured (render-spike never scrolled) | p50 16.2-16.4ms, p99 19.4-29.2ms (3 runs, n=100) | New, real, non-trivial cost — not under the 5ms target |

The random-position row needs an honest caveat, not a rounded-away one: with a 34-60 line viewport against 50,000 lines, a uniformly random edit position lands inside the visible window only ~0.07-0.1% of the time — across three 2000-iteration runs, only 0-1 edits actually landed in-window, so that number is really measuring "how cheap is a genuine no-op," not real reshape cost. A **new, dedicated cursor-adjacent typing benchmark** was added specifically to close that gap: it types sequentially at the cursor (which starts at line 0 and never scrolls during this phase), so every single edit is a real in-window reshape regardless of the surrounding document's size. That number — not the random-position one — is the honest answer to "does virtualization help realistic typing," and it does: p99 3.5-3.9ms is both better than render-spike's own p99 at the same document size and reliably under §39.1's 5ms p99 target, which render-spike's own report said was "not reliably met."

**Real visual verification** (screenshot + `enigo`-based synthetic OS input, the same methodology already established for render-spike/ui-shell-spike): opened a real file (`crates/spartan-buffer/src/lib.rs`), confirmed the real detected language printed to stdout (`rust`, `tree-sitter-rust`, LSP and DAP both present), confirmed real file content rendered on screen, typed two real lines via OS-level synthetic keyboard input (not the scripted bench path), confirmed the caret rendered at the correct on-screen position between the newly-typed text and the file's original first line, scrolled forward two pages and confirmed the visible content changed to a different part of the document, scrolled back and confirmed the original content — now including the injected text exactly where it was typed — rendered identically to the pre-interaction screenshot.

**A real bug found only by running the scripted benchmark, not by inspection**: the first version of the benchmark's exit-report logic checked "have we reached the target count" on every `RedrawRequested` frame with no latch, so once a phase's target was reached, its "benchmark complete" report printed on every subsequent frame instead of once — dozens of duplicate prints before the fix. Fixed with explicit one-shot `*_bench_reported` flags per phase.

14 new headless tests (`crates/spartan-editor-core/tests/viewport_and_language.rs`, no GPU/window/display needed) cover `Viewport`/`windowed_text`/`to_local_line` slicing and the language-registry lookup wiring, alongside the pre-existing `spartan-buffer`/`spartan-languages` suites. `cargo clippy --workspace --all-targets --release -- -D warnings` and `cargo fmt --check` are clean across the whole workspace including this crate.

**What this does not confirm.** §39.1's <100ms cold-open target is still not met (575-620ms, ~6x over) — meaningfully closer than render-spike's 897-1298ms, but not closed. Scrolling is a new, real, non-trivial cost (p99 19-29ms) that stacks against the same latency target edits are measured by, and is unaddressed. Viewport auto-follow-cursor is not implemented — a user typing enough newlines near the bottom edge of the visible window will move the cursor off-screen with no auto-scroll, a named, deliberate simplification, not a bug found later. `visible_lines` is computed once from the window's initial size and never recomputed on resize. Tree-sitter, real LSP/DAP process spawning (only the registry lookup is real), Leo, and any UI chrome (scrollbar, tabs, panels) remain entirely unbuilt.

### 75.6 Real LSP Wiring — `crates/spartan-editor-core`'s First Live Language Server

§75.3 and §75.5 both named "wiring the registry's `lsp_command`/`dap_command` fields into the already-proven `lsp-spike`/`dap-spike` clients" as the next real gap. This increment does the LSP half — a real, live `rust-analyzer` session for the currently open file, spawned, kept in sync with real edits, and reporting real diagnostics — scoped deliberately to diagnostics only, not DAP: no debugging UI (breakpoints, stepping, variable inspection) exists anywhere in this crate yet, so DAP needs substantially more new interaction design than LSP diagnostics do (which only need to be printed to stdout, the same "real, but no UI yet" pattern already established for language detection itself). DAP wiring is explicitly deferred to a later pass, not attempted here.

**What was built.** `src/lsp.rs` promotes `spikes/lsp-spike`'s `LspClient` and `DidChangeDebouncer` verbatim — the same real, synchronous, hand-rolled-JSON-RPC-over-stdio client already proven against two independent servers (rust-analyzer, pyright-langserver) by that spike's own tests. The genuinely new piece is `src/lsp_session.rs`: `LspClient`'s calls block synchronously, some (indexing) for up to 90 seconds, so they can never run on the render thread without freezing the whole editor. `LspSession` runs the entire session — subprocess spawn, the initialize/didOpen handshake, and every subsequent didChange dispatch — on its own dedicated OS thread; `LspSession::spawn()` itself only blocks long enough to start the subprocess (near-instant), so the protocol handshake and indexing wait never add to cold-open time. The debounce timer (150ms idle default, §2.3) stays on the render thread instead, since it's just cheap `Instant` comparisons already polled every `AboutToWait` tick; only when it actually fires does the render thread pay the one real cost — stringifying the current buffer — and hand that single snapshot to the background thread via a single-slot mailbox (`Mutex`+`Condvar`), not a channel. That mailbox design exists specifically because a plain `mpsc` channel gives the sender no way to evict a stale queued value: without it, several debounce firings during the up-to-90s cold indexing wait would all queue up and dispatch in order once the thread is free, redundantly re-analyzing stale snapshots before diagnostics ever catch up to real time. `language::find_project_root` (new, small) walks up a file's ancestor directories looking for any of the language profile's own `marker_files` — reusing data `spartan-languages` already provides rather than duplicating its marker-file logic — to find the project root an LSP server needs for real multi-file analysis.

**A real, load-bearing correctness fix beyond what the spike's own tests covered**: `LspClient::wait_real_diagnostics` only ever resolves on a *non-empty* diagnostics array, by design — that's what the spike's own tests need. Reusing it verbatim for live edits would mean a fixed error could never be reported as fixed; the stale diagnostics would stay printed forever. `lsp_session.rs`'s dispatch loop instead calls `wait_notification` directly for every dispatch after the first, reporting the real array length including zero ("0 diagnostics — clean"). This also means `did_change_full` — the one piece of the spike's "proven" client its own tests never actually exercised against a real server — got its first real exercise in this pass's new integration test, not an untested corner quietly inherited from the spike.

**Real, executed verification, not asserted.** A new self-skipping integration test (`crates/spartan-editor-core/tests/lsp_integration.rs`, skips if `rust-analyzer` isn't on `$PATH`) spawns a real `LspSession` against a real generated Cargo fixture with a deliberate `E0308` type error, confirms a real non-empty diagnostic is reported (2.49s wall-clock in the run that produced this section — indexing was fast for this tiny fixture, well under the 90s budget planned for), then calls `notify_edit` with corrected text and confirms diagnostics really update to empty. Separately, a live binary run against a real generated fixture (screenshot + `enigo` synthetic OS input, the same methodology already established for render-spike/ui-shell-spike) confirmed: real language detection and LSP session start printed to stdout, a real diagnostic ("line N: error - expected i32, found &'static str") printed after real rust-analyzer analysis, then — after using synthetic keyboard input to type `//` at the cursor, commenting out the file's one line — the real diagnostics update printed "0 diagnostics — clean" within one debounce cycle, confirmed both in the process log and via a follow-up screenshot showing the commented-out line rendered on screen. The 50k-line `--synthetic:` benchmark was re-run afterward and showed no measurable change from the previous pass (cold-open 589.46ms, cursor-adjacent edit p99 3.77ms, scroll p99 21.25ms — all within the prior run-to-run variance), confirming the LSP wiring — which never spawns for `--synthetic:` fixtures, since they have no real project root — adds no cost to the existing benchmark path.

**What this does not confirm, named plainly.** The live-edit visual verification's "fix" was a `//` comment-out, not a realistic code correction — this crate still has no cursor-movement input beyond PageUp/PageDown (a limitation already named in §75.5), so the only edit reachable purely by typing from the cursor's fixed start-of-file position was prepending text, not editing the erroring line's actual contents; the underlying wiring being verified (a live edit causing a real, updated diagnostics report) is the same either way, but the specific edit demonstrated is a workaround for that pre-existing input limitation, not a demonstration of realistic in-place code fixing. A standalone file with no discoverable marker file in any ancestor directory falls back to single-file mode (`find_project_root` returns `None`), which rust-analyzer still handles but with meaningfully worse diagnostics — a named, real limitation, not silently absorbed. `LspSession::shutdown()`'s bounded waits (~7s worst case) will visibly delay window close if triggered while a request is in flight — mitigated with a printed status line so it reads as a known wait, not a hang, but not eliminated. No hover, no completion, no DAP, and no diagnostics UI (gutter squiggles, a problems panel) exist — diagnostics are printed to stdout only, matching how detected language itself is also just printed. A crashed or hung language server is never detected or restarted mid-session.

### 75.7 Closing Two Named Viewport Gaps: Auto-Scroll-to-Cursor and Resize-Aware Recomputation

§75.5 named two real, specific limitations of the viewport-virtualization work: no auto-scroll-to-cursor (typing enough newlines near the bottom edge of the visible window moved the cursor off-screen with no follow), and `visible_lines` computed once at startup and never recomputed on resize (so a resized window's viewport size silently drifted from what was actually on screen). Both are small, well-understood fixes to existing architecture — not new subsystems — so this pass implemented them directly rather than through another research/plan cycle.

**What was built.** `Viewport::ensure_visible(doc_line, doc_len_lines) -> bool` (new, in `viewport.rs`) scrolls minimally so a given document line becomes visible: if it's above the window it becomes the new first visible line, if below the new last visible line, clamped to the document's valid scroll range. `main.rs`'s real (non-benchmark) keyboard-input path now calls this after every edit, using the cursor's post-edit line — if the viewport had to scroll, one full reshape against the new window covers both the scroll and the edit's own visual change, avoiding a wasted double reshape when they coincide (the common case: pressing Enter near the bottom edge is simultaneously a structural edit and a scroll-triggering one). The scripted benchmark's edit paths (`AboutToWait`) are deliberately left unchanged — the cursor-adjacent benchmark's own methodology already depends on the cursor never leaving the window, so wiring auto-scroll there would only ever be a no-op, not a meaningful change. Separately, `WindowEvent::Resized` now recomputes `visible_lines` from the new height (the same formula used at startup) and, if it changed, calls `ensure_visible` for the current cursor position before reshaping the window against the new, correctly-sized slice.

**Real, executed verification.** Three new headless tests (`ensure_visible` scrolling up, scrolling down, and a no-op when already visible) plus a defensive-clamp test for an intentionally out-of-range line (a case a real cursor position should never produce, but the guard is real code, not asserted-only). One of the first versions of that clamp test was wrong, not the implementation — the test assumed clamping was reachable for a valid last-line target, but the arithmetic (worked through by hand, not just re-run) shows the clamp can never actually engage for any valid document-line input, only for a genuinely out-of-range one; the test was corrected to check what the code can actually do, not a scenario invented in the plan. Real visual verification (screenshot + `enigo` synthetic OS input, same methodology as prior passes): sent 45 real `Enter` keypresses into a 200-line synthetic file's editor (more than the ~34-line visible window) and confirmed via screenshot that the caret stayed visible at the bottom of the window rather than scrolling off-screen. Separately, sent a real Win32 `SetWindowPos` call to shrink the window to 900×400 and confirmed via screenshot that the content reflowed and the caret remained correctly positioned and visible after the resize. The 50k-line `--synthetic:` benchmark was re-run afterward and showed no regression (cursor-adjacent p99 4.25ms, scroll p99 27.41ms — both within the prior passes' run-to-run variance; the benchmark paths don't exercise the new auto-scroll code at all, by design, so this mainly confirms the resize-recomputation code adds no cost on the hot path it doesn't touch).

**What this does not confirm.** Auto-scroll always re-centers the cursor at the window's edge (first or last visible line), not with any surrounding context margin — a real editor convention (keeping a few lines of context above/below the cursor) that this pass didn't attempt. Horizontal scrolling/wrapping for long lines remains entirely unaddressed. The resize fix recomputes line *count* but not the existing named simplification that `TEXT_ORIGIN_X`/`TEXT_ORIGIN_Y` and font metrics stay fixed regardless of window width — a very wide or narrow resize isn't specifically tested here, only a height change.

### 75.8 Real DAP Wiring — `crates/spartan-editor-core`'s First Live Debug Session

§75.3 named wiring both `lsp_command` and `dap_command` into the already-proven `lsp-spike`/`dap-spike` clients as the next real gap; §75.6 did the LSP half. This pass does the DAP half: real breakpoints, a real hit, real continue/step commands, and real stack/variable inspection, using `spikes/dap-spike`'s already-proven `DapClient` — tested against two independent real adapters (`lldb-dap` for Rust, `debugpy` for Python).

**Two real scoping decisions, made explicit rather than discovered as gaps later.** First: `spartan-languages`'s `dap_command` field only names the debug *adapter* binary — there is no existing convention anywhere in the registry for "how to build this project into a debuggable binary." Building that (real `cargo build` integration, parsing its output for a binary path, handling failures) is separate, substantial scope. This pass takes a pre-built debug binary path as an explicit CLI argument (`--debug-binary:<path>`) instead, the same way the crate already takes a fixture path. Second: `dap-spike` also proves rope-anchored breakpoint persistence (the literal §39.2 success criterion, "breakpoint survives an edit that shifts line numbers") — wiring that up for real would need raw byte-level edit details (`EditorView`'s edit methods only report `EditEffect::Line`/`Structural`, not insertion offsets/lengths) and a char-to-byte bridge that doesn't exist in this crate's public API yet. This pass uses line-number breakpoints instead — the exact fallback §39.2 itself sanctions ("if rope-anchored breakpoint persistence proves too complex, fall back to line-number-based breakpoints for v1... not a launch blocker").

**What was built.** `src/dap.rs` promotes `DapClient` from the spike verbatim (`spawn`, `request`, `wait_event`, `launch_and_break`, `stack_trace`, `scopes`, `variables`, `continue_`, `shutdown`), plus two small new methods matching `continue_()`'s exact one-line `request()`-wrapper shape: `step_over` (DAP "next") and `step_into` (DAP "stepIn") — standard protocol commands the spike itself never needed. `src/dap_session.rs` is the new orchestration layer, and its threading design is deliberately different from `LspSession`'s mailbox: LSP coalesces edits because only the *latest* snapshot matters, but every debug command a user issues (continue, step-over, step-into) is a discrete, ordered action that must execute exactly once — a plain ordered `mpsc::channel` is used instead, so nothing is ever dropped or coalesced. `DapSession::launch()` never blocks the caller (subprocess spawn + thread start only; the launch/breakpoint handshake happens in the background, matching `LspSession`'s "handshake off the render thread" design), then loops on incoming commands, dispatching each to the matching `DapClient` method and waiting for the resulting event via a small helper that tries "stopped" first and falls back to "exited" — safe because `wait_for`'s existing message-buffering (already relied on throughout `dap.rs`/`lsp.rs`) re-queues anything that doesn't match a given wait for a later, differently-named one. `main.rs` wires `F9` (toggle a line-number breakpoint at the cursor), `F5` (launch, or continue if already running), `F10`/`F11` (step-over/step-into), printing stop/exit/error updates to stdout — no debug UI exists yet, the same "real, but no UI" pattern already established for language detection and LSP diagnostics.

**A real environment constraint hit during verification, and how it was resolved without faking anything.** `lldb-dap`/`lldb-dap-18` are not installed on the machine this pass ran on (confirmed by an actual `find_adapter()`-equivalent search, not assumed) — a real difference from an earlier session's environment where `dap-spike`'s own Rust tests had already run. Rather than skip real verification entirely or claim untested code works, this pass added a second integration test (`tests/dap_python_cross_language.rs`, mirroring `dap-spike`'s own cross-language test) exercising the identical `DapSession`/`DapClient` code path against real `debugpy`, which *is* installed here. It passed for real: a real breakpoint hit on the correct line with the correct local variable value, a real `StepOver` that landed on the actually-correct next line (not asserted, checked), and a real `Continue` that ran the program to a real, observed exit. This is exactly the same "prove the design isn't adapter-specific" logic `dap-spike` itself used to justify testing against two adapters, applied here for a practical reason (only one adapter was actually available) rather than a design-validation one. The primary Rust/`lldb-dap` test (`tests/dap_integration.rs`) correctly self-skips on this machine rather than failing.

**A second real, unplanned finding, also from actually trying it, not from inspection**: `spartan-languages`'s own `languages.toml` entry for Python's `dap_command` (`program = "debugpy"`) is not directly invocable — `debugpy`'s adapter is `<python> -m debugpy.adapter`, a module invocation, not a standalone binary on `$PATH`. `dap-spike`'s own Python test already worked around this with a generated wrapper script; this pass's new `debugpy` test does the same thing at the test level, but `main.rs`'s real, profile-driven dispatch path has no such wrapper-generation step, so it could not be used to verify the live GUI's `F9`/`F5` keybindings against a real, stopping debug session in this environment for *either* language — Rust because the adapter isn't installed, Python because the registry entry as written isn't directly runnable. What **was** verified live, for real, through the actual product binary: `F9` correctly toggles and prints a line-number breakpoint at the cursor; `F5` correctly attempts a real launch and, when the configured adapter genuinely cannot be spawned, reports a clean `DAP error: failed to spawn lldb-dap: program not found` rather than hanging or crashing — confirmed via both the process log and a follow-up screenshot showing the editor still fully responsive afterward. The 50k-line `--synthetic:` benchmark was re-run and shows no regression (DAP never spawns for synthetic fixtures or without `--debug-binary:`).

**What this does not confirm.** The live GUI's breakpoint-hit/step/continue flow was not observed against a real, actually-stopping adapter *through the real product binary* in this environment — only through the two new integration tests, which exercise the identical `DapSession` code but not `main.rs`'s own keybinding dispatch end-to-end. The Python registry entry's non-invocable `dap_command` is a real, pre-existing gap this pass did not fix (fixing it well would mean either a richer `CommandSpec` model or a wrapper-generation step in `main.rs` itself — both real, separate work). No live breakpoint changes once a session is running. No rope-anchored breakpoint persistence (line-number breakpoints will silently point at the wrong line after an edit that shifts lines above them — a real, named limitation, not a silent one). No DAP UI (breakpoint gutter markers, a variables panel — stdout only). No build-system integration (`--debug-binary:<path>` must already exist). `continue_` and the new `step_over` are exercised for real by this pass's own debugpy test — genuinely new coverage, since the spike's own tests never call either. `step_into` (wired to `F11`) compiles and follows the identical pattern as the now-tested `step_over`, but nothing in this pass's tests or live verification actually issues it — an honest, remaining gap, not glossed over.

### 75.9 Cold-Open Investigation: a Real Sub-Breakdown, One Fix That Didn't Help, One That Did

§75.5-§75.8 all named the ~575-620ms cold-open number (~6x over §39.1's <100ms target) without ever measuring which step actually causes it. This pass added a real, permanent per-step timing breakdown to `main.rs` (printed alongside the existing single-number cold-open report) and used it to find and evaluate two real, targeted fixes — one that measurably helped, one that didn't, both reported honestly rather than only keeping the flattering result.

**The real breakdown**, from repeated instrumented runs against the 50k-line synthetic fixture on the same Intel UHD 620 / Vulkan / Windows-GNU hardware every prior pass used:

| Step | Cost |
|---|---|
| arg parsing / fixture load / language detect / LSP spawn | ~15-33ms |
| `winit::EventLoop::new()` | ~4-5ms |
| window creation | ~67-181ms |
| `GpuState::new()` (wgpu instance/adapter/device/surface) | ~220-433ms — the largest single cost |
| `TextState::new()` (cosmic-text `FontSystem::new()` + glyphon atlas) | ~93-97ms before this pass's fix, ~2-2.5ms after |
| initial `reshape_window()` | ~10-18ms |
| `CursorRenderer::new()` | ~0.3-0.4ms |
| first `RedrawRequested` (surface acquire -> present) | ~71-121ms |

**Fix attempted and reverted (a real negative result, not hidden)**: `wgpu::Instance::new()` with `Backends::all()` probes every graphics API loader (Vulkan, DX12, DX11, GL) even though `adapter_info.backend` has reported `Vulkan` on every single run of this project, on every machine, so far. The hypothesis was that skipping the other three backends' probe cost by trying a `Backends::VULKAN`-only instance first (falling back to `Backends::all()` only if that finds no adapter) would cut `Instance::new()`'s real, measured ~221-306ms cost. Implemented, then measured across 5 repeated runs: the Vulkan-only path's timings (261-334ms) fully overlap the original range — no measurable improvement. Reverted rather than keep unproven complexity around; the ~220-300ms cost is apparently intrinsic to Vulkan loader/ICD initialization itself on this hardware, not to probing other backends. A real lesson worth keeping: a plausible-sounding performance hypothesis (backed by real reasoning about what the API does) still needs to be measured, not just implemented and assumed correct — this project's own rule, applied to itself.

**Fix that measurably helped**: cosmic-text's `FontSystem::new()` scans and parses every font on the system — a real, measured ~93-97ms cost with no actual dependency on the GPU device/queue `TextState::new()` otherwise needs (only `TextAtlas::new()` does). `main.rs` now spawns `FontSystem::new()` on its own OS thread concurrently with `GpuState::new()`'s async GPU setup, joining the result just before constructing `TextState`. Real effect, measured across 5 runs: `TextState::new()`'s own cost dropped from ~93-97ms to ~2-2.5ms (the font scan now overlaps with the ~220-430ms GPU setup instead of running after it), and overall cold-open dropped to a 467-715ms range (average ~577ms) from the prior ~570-810ms range this same investigation's own baseline runs showed — a real, if modest, improvement, since `GpuState::new()`'s own run-to-run variance still dominates the total.

**A real methodological finding from verifying this fix, worth recording plainly**: an initial re-run of the full edit/cursor/scroll benchmark after this change appeared to show a real regression (cursor-adjacent p99 4.65-4.73ms and scroll p99 41.0-41.5ms, both well outside every previously-recorded range for this crate). Rather than either dismissing this as noise or reporting it as a real regression without checking, it was investigated properly: the previous git-committed binary (pre-cold-open-changes) was rebuilt and run under the exact same conditions, moments later — and showed the *same* kind of numbers a fresh, cold system would, not the elevated ones. A controlled, order-alternating A/B/A/B comparison (old, new, new, old, each run fresh) then showed all four runs statistically indistinguishable (cursor-adjacent p99 3.07-3.33ms, scroll p99 18.2-22.1ms) — confirming the earlier elevated numbers were transient system noise from the many rapid rebuild-and-run cycles this investigation itself had been doing, not a real effect of the code change. This is the same "don't assume, verify" discipline applied to a case where the *tempting* conclusion (blame recent code) turned out to be wrong.

**What this does not confirm.** §39.1's <100ms cold-open target remains far from met — even the best measured runs (~467ms) are ~4.7x over, and `GpuState::new()`'s own ~220-430ms is now the clear, dominant, largely-unaddressed remaining cost (window creation and the first `RedrawRequested`'s ~70-180ms combined are the next largest). No further wgpu-specific optimization avenues (surface format negotiation shortcuts, deferred/lazy adapter enumeration, a persistent adapter cache across process runs) were investigated in this pass. The permanent per-step breakdown print adds a small, real amount of `Instant::now()`/formatting overhead to every cold-open (not separately measured, assumed negligible relative to the multi-hundred-millisecond costs it reports on).

### 75.10 Real DAP Build-System Integration — Closing §75.8's Named Gap

§75.8 named a real, deliberate scope cut: `spartan-languages`'s `dap_command` field only names the debug *adapter*, not how to build a debuggable binary, so that pass required a pre-built `--debug-binary:<path>`. This pass closes that gap for real, for the one build system this project's own Tier 1 language set actually needs first: Cargo.

**What was built.** `crates/spartan-editor-core/src/build.rs`'s `build_debug_binary(project_root) -> BuildResult` runs a real `cargo build --message-format=json` and parses cargo's own real, documented JSON message stream — `compiler-artifact` (filtered to `target.kind` containing `"bin"`, since a lib-only crate has no debuggable executable) for the real `executable` path, `compiler-message` entries at `level: "error"` for real rendered diagnostics, and `build-finished.success` for the overall verdict. The exact JSON shape was confirmed by running a real `cargo build --message-format=json` against a real fixture crate before writing any parsing code — both a clean build and a real `E0308` compile error — rather than coded against remembered/assumed documentation. `main.rs`'s language-detection block now captures `dap_build_info` (adapter command, project root, source path) whenever no explicit `--debug-binary:` was given but the detected profile's `build_systems` includes `"cargo"` and a real `Cargo.toml` exists at the discovered project root. `F5` now has three real cases instead of two: continue an existing live session, launch immediately with an explicit `--debug-binary:`, or — new — run a real build on its own thread (never blocking the render loop, matching the `LspSession`/`DapSession` "never block" pattern even though this itself is a one-shot operation, not an ongoing session) and launch with the resulting real binary once it completes.

**A second, small real fix bundled into the same pass**: `DapSession::is_finished()` (wrapping `JoinHandle::is_finished()`) lets `F5` tell a genuinely-over session (the debuggee ran to a real exit, or the launch sequence failed) apart from a live one. Before this, pressing `F5` again after a debug session ended would silently try to `send_command` into a channel nothing was listening on anymore — harmless (the send just fails silently), but useless: no rebuild, no relaunch, no feedback. Now a finished session is dropped and `F5` correctly starts fresh — a real fix to the edit-debug-rebuild loop this pass's own build integration is meant to enable, not just to the new code path.

**Real, executed verification.** Two new tests (`tests/build_integration.rs`) run a real `cargo build` against real generated fixture crates (the same temp-dir-outside-the-workspace pattern already established for `lsp_integration.rs`/`dap_integration.rs`, since cargo — like rust-analyzer — refuses a manifest that's an undeclared descendant of this workspace's own root): one confirms a real successful build reports a real executable path that actually exists on disk, the other confirms a real deliberate type error reports the real rendered `E0308` diagnostic. Live, through the actual product binary: opened a real generated Cargo project's `src/main.rs` with no `--debug-binary:` given, confirmed `"DAP ready: lldb-dap via a real cargo build"` printed (the new capture path firing correctly), pressed `F9` then `F5` via synthetic input, confirmed a real `cargo build` ran and reported the real, correct executable path, confirmed the freshly-built binary genuinely exists on disk (4.8MB, real mtime), and confirmed the flow correctly proceeded to attempt a DAP launch with that binary — failing gracefully with the same honest, non-crashing `lldb-dap not found` error already established in §75.8, since this machine still has no `lldb-dap` installed. The 50k-line `--synthetic:` benchmark was re-run afterward and shows no regression (cursor-adjacent p99 3.01ms, scroll p99 18.58ms, both within established range) — build integration never engages for synthetic fixtures.

**What this does not confirm.** The full build-then-hit-a-real-breakpoint flow was not observed end-to-end through the live GUI in this environment, for the same reason as §75.8: no `lldb-dap` installed here. (The underlying `DapSession` launch/breakpoint/continue/step path was already verified for real against `debugpy` in §75.8 and is unchanged by this pass — only how the binary gets built is new.) Only Cargo is wired — a language whose `build_systems` names something else (`npm`, `poetry`, `gradle`, ...) still requires an explicit `--debug-binary:<path>`, matching §75.8's original scope, now narrowed rather than fully closed. No incremental-build awareness beyond what `cargo build` itself already does (no "skip build if nothing changed" logic in `main.rs` — every `F5` press with no live session re-invokes cargo, which is itself fast on a no-op rebuild but still a real subprocess spawn every time). No build cancellation if the user presses `F5` again mid-build (the existing build simply finishes and its result is used; a second `F5` press during that window is rejected with a printed message, not queued or merged).

### 75.11 Real Tree-Sitter Syntax Highlighting — Rust Only, Windowed

Every `LanguageProfile` has carried an unused `tree_sitter_grammar: String` field since §75.5 ("tree-sitter stays unwired" was a standing, named gap through §75.10). This pass wires it up for real, for Rust first — matching every prior pass's own precedent (LSP started with `rust-analyzer`, DAP with `lldb-dap`).

**A real, load-bearing API limitation, found before writing any product code, that forced a scope decision.** `tree_sitter_highlight::Highlighter::highlight()`'s public convenience API always scans its *entire* input — confirmed by reading the actual installed `tree-sitter-highlight-0.25.10` source, which hardcodes the scan range to `0..usize::MAX`, not by assuming from documentation. Since every other subsystem in this crate is already scoped to the visible viewport specifically to keep cost independent of document size, and this API has no cheap way to restrict to a sub-range, this pass parses and highlights **only the currently-windowed text** (`viewport::windowed_text()`, the same ~34-60 line slice everything else uses), never the whole document. This is a real, named limitation: a multi-line construct (a block comment, a raw string) that starts above the visible window will be misinterpreted within it, since the parser has no context above the window's first line. The correct fix — a persistent, incrementally-updated whole-document `tree_sitter::Tree` (`Tree::edit()` + `Parser::parse(text, Some(&old_tree))`) combined with a byte-range-restricted query pass using the lower-level `tree_sitter::{Query, QueryCursor}` API instead of the convenience wrapper — is real, valuable future work, named here rather than attempted under time pressure.

**What was built.** `crates/spartan-editor-core/src/highlight.rs`'s `Highlighter::rust()` builds one real `HighlightConfiguration` from `tree_sitter_rust::LANGUAGE` and the grammar's own bundled `HIGHLIGHTS_QUERY` constant (no `.scm` file needed for this pass); `Highlighter::highlight(source) -> Vec<HighlightSpan>` runs a real parse + highlight-query pass and walks the resulting `HighlightEvent` stream with a color stack (push on `HighlightStart`, pop on `HighlightEnd`, use the top of stack for `Source` events) to handle real nesting (e.g. a macro call inside a larger expression). A deliberately small, fixed six-name "theme" (`keyword`, `string`, `comment`, `function`, `type`, `constant`) is passed to `configure()`; any capture name outside it renders in cosmic-text's own default color. `TextState::set_text_highlighted()` (`text.rs`) renders the result via `cosmic_text::Buffer::set_rich_text` + `AttrsList`-backed per-span `Attrs::color(...)` — the real rich-text API, confirmed by reading the actual installed `cosmic-text-0.10.0` source, not docs.rs. `main.rs`'s `reshape_window()`/`apply_edit_effect()` both take a new `highlighter: Option<&mut highlight::Highlighter>` parameter; a `Highlighter` is constructed only for real (non-`--synthetic:`) files whose detected profile's `tree_sitter_grammar == "tree-sitter-rust"`, matching the LSP/DAP precedent of naming an unhandled grammar rather than silently guessing at it. **The per-line fast path (`set_line_text`, used for `EditEffect::Line`) is deliberately bypassed whenever a `Highlighter` is active** — a single edited line can't be correctly re-highlighted in isolation (is this token inside a string that started on a previous line within the window?) — so any edit to a highlighted file routes through the full windowed re-parse-and-reshape path, same as a `Structural` edit already does.

**A real bug caught by the visual verification itself, not by inspection.** The first live screenshot showed numeric literals (`0`, `3`, `4`) rendering in the default, uncolored text — the `"number"` entry in `HIGHLIGHT_NAMES` never matched anything. Reading `tree-sitter-rust`'s actual bundled `queries/highlights.scm` showed why: it captures integer/float literals as `@constant.builtin`, never `@number`. Reading `tree_sitter_highlight::HighlightConfiguration::configure()`'s real matching rule (also from its installed source) showed the fix: a configured name matches a capture if every one of the configured name's dot-separated parts is present among the capture's own dot-separated parts — so `"constant"` (the fix, replacing `"number"`) matches both plain `@constant` and `@constant.builtin`, the same mechanism that already let the existing `"function"` entry match `@function.macro` for `println!`. A new headless test (`a_real_integer_literal_gets_a_real_span`) locks this in.

**Real, executed verification.** Three (now four, after the fix above) headless tests in `highlight.rs` confirm real byte-accurate spans for a keyword, a string literal, a line comment, and an integer literal against the crate's own real `Highlighter` — one of these (the comment test) itself caught a real off-by-one assumption during development (the comment span excludes the trailing newline, a separate token) by actually running it. Live visual verification: a real six-construct Rust fixture (struct, function, comment, string, numeric literals, macro call) opened through the actual binary and screenshotted twice (before and after the `constant` fix) — the "after" screenshot confirms all six categories render in real, visually distinct colors: comments blue-gray, keywords purple (`struct`/`fn`/`let`/`as`), types orange (`Point`/`i32`/`f64`), functions blue (`distance`/`main`/`sqrt`/`println!`), strings green, and numeric literals orange (sharing the `constant` color). Bringing the target window to the foreground for the screenshot needed a real fix mid-pass: a plain `SetForegroundWindow` silently no-oped because another window (a browser) had received more recent input, a real instance of Windows' focus-stealing prevention — worked around by briefly forcing `HWND_TOPMOST` (which has no foreground-rights requirement) before calling `SetForegroundWindow`, then releasing it with `HWND_NOTOPMOST`.

**A new, real, honestly-reported latency cost.** Because the per-line fast path is bypassed for highlighted files, cursor-adjacent typing latency was measured for a real 899-line Rust file (this crate's own `main.rs`) both with highlighting active and, for a same-content baseline, with the identical text saved under an unrecognized extension (so no `Highlighter` attaches and the fast path stays active) — isolating the highlighting-bypass cost from any document-content difference. Two runs each: highlighted p50 5.2-5.9ms / p99 28.7-34.3ms / max 67-102ms; unhighlighted (same content) p50 2.8-3.1ms / p99 5.8ms / max 15-17ms. This is a real, reproducible ~1.9x p50 / ~5x p99 cost, not noise — reported plainly rather than rounded away, exactly the trade-off this section's own plan named up front as something that must be measured, not assumed acceptable. The 50k-line `--synthetic:` benchmark (which never constructs a `Highlighter`) was re-run and, after a real A/B against the prior committed (pre-tree-sitter) binary confirmed the day's ~7-9ms p99 baseline itself had shifted from an earlier session's recorded 3.5-3.9ms due to environmental variance (matching §75.9's own established methodology for telling a real regression apart from noise) rather than from this pass's changes, shows no regression attributable to this pass.

**A second real bug, this one in the benchmark harness itself, found while measuring the above.** Passing a literal `0` for the scripted scroll-benchmark phase (intending "skip this phase") instead makes it report complete and call `elwt.exit()` on the very first frame, since `0 total_recorded() >= 0` is trivially true immediately — cutting off any later-ordered phase (here, a 2000-iteration cursor benchmark) before it had a chance to run. The correct way to skip a phase is to omit its argument entirely (giving `None`, which the `if let Some(total) = ...` guard never enters), not to pass `0`. Not fixed in this pass (a pre-existing harness quirk, not part of the shipped product code), but named here so a future benchmark run isn't silently truncated by the same mistake.

**What this does not confirm.** Only Rust is wired — any other `tree_sitter_grammar` value is named and left unhighlighted, matching every prior pass's precedent for an unhandled language. Only six capture names have colors; the rest of `tree-sitter-rust`'s bundled query (there are dozens of capture names) renders in the default color. No incremental re-parsing — every reshape re-parses the whole visible window from scratch, even for a single-character edit. No syntax highlighting above/below the visible window, and (named above) real misinterpretation risk for multi-line constructs that start above it. No configurable theme — the six colors are hardcoded. The per-line-fast-path bypass's real latency cost (above) is unaddressed, not just unmeasured — a `Tree`-based incremental design would very likely close most of it, but that's the same future work already named for the whole-document-context limitation.

### 75.12 First Real Linux-Container Verification Pass — a Real Cross-Platform Regression Found and Fixed in §75.8's Python DAP Port

Every prior §75.x pass ran on a Windows machine with a real GPU (Intel UHD 620). This pass ran in a Linux container instead — a genuinely new environment for this project's own history, confirmed rather than assumed (`ls /dev/dri` empty, no `glxinfo`) to have no GPU/display at all. That rules out re-running anything GUI/visual (`render-spike`, `ui-shell-spike`, `spartan-editor-core`'s own product binary, any screenshot workflow) here — this pass is a pure `cargo test --workspace --release` + `clippy`/`fmt` verification pass, not a feature increment, and is reported as exactly that rather than stretched into more.

**Two real, environment-specific build gaps found and fixed before the workspace would even compile.** `gdk-sys` (pulled in transitively by `winit`'s Wayland/adwaita-decoration support) failed to build without GTK3's `pkg-config` files; `javascriptcore-rs-sys` (pulled in by `ui-shell-spike`'s `wry` dependency) failed the same way without WebKitGTK's. Both are real, correct build requirements for a from-scratch Linux desktop toolchain, not code bugs — no prior pass had ever surfaced them because every prior pass ran on Windows, where these crates resolve to entirely different backends. Fixed by installing the corresponding system dev packages (`libgtk-3-dev`, `libwebkit2gtk-4.1-dev`) — a machine-local fix, not a repo change.

**Once compiling, the full workspace suite passed for real** (all tests across all 6 spikes + 3 real crates), including new territory for this project's own history: this machine has a real, installed `lldb-dap-18` (`which lldb-dap-18` succeeds), unlike every session §75.8–§75.11 documented, where lldb-dap was either absent (most sessions) or (one session) present without a display for full GUI verification. `spikes/dap-spike/tests/dap_integration.rs`'s three tests and `crates/spartan-editor-core/tests/dap_integration.rs`'s one test all ran against the real adapter rather than self-skipping.

**A real regression found only by getting both real DAP adapters into the same session for the first time.** `python3` and `pip` were present but the `debugpy` package was not; it was installed for real (`pip3 install debugpy`) specifically to get the same dual-adapter DAP proof §75.8 already established, in one session this time instead of split across two. That surfaced a real, previously invisible bug: `crates/spartan-editor-core/tests/dap_python_cross_language.rs`'s `debugpy_wrapper_command()` had silently dropped `spikes/dap-spike`'s own `#[cfg(not(windows))]` branch when §75.8 ported/adapted the function — only the Windows `.cmd`-batch-file branch survived the port. A `.cmd` file has no executable bit and no interpreter on Linux, so `DapClient::spawn` failed to launch it, and the test panicked for real (`expected the first update to be Stopped, got an Error or Exited instead`) instead of exercising the breakpoint/step/continue flow it claims to. This was invisible until now for a specific, real reason: every prior session that had `debugpy` available was a Windows session (where the surviving `.cmd` branch is correct), and every prior non-Windows-shaped session lacked `debugpy` entirely — this is the first session with both a non-Windows environment and a real `debugpy` install at once.

**Fix.** Restored `dap-spike`'s original `#[cfg(windows)]` / `#[cfg(not(windows))]` split verbatim into the port (the Unix branch writes a `#!/bin/sh` wrapper and `chmod`s it `0o755`, exactly as the un-regressed original already does), with a doc comment naming why the regression was invisible until this exact environment combination. Re-run: passes for real in ~6.5s, including the `StepOver` (line 2 → line 3) and `Continue`-to-a-real-exit assertions the test already contained but had never actually exercised end-to-end, since the wrapper couldn't even spawn before this fix.

**Real, executed verification.** `cargo test --workspace --release` — full green, before and after the fix for every test except the one it fixed. `cargo clippy --workspace --release --all-targets` and `cargo fmt --all -- --check` both clean, no warnings, no diff, re-confirmed after the fix. `spikes/dap-spike`'s own Python cross-language test (unaffected by this bug, already had the correct platform split) also re-confirmed passing for real against the same `debugpy` install.

**What this does not confirm.** No GPU/display in this environment, so nothing GUI/visual was re-verified — this pass adds no new screenshot or live-binary evidence, only headless-test evidence. `debugpy` is a system package installed for this session's own verification, not vendored or committed to the repo; a future session without it will still correctly self-skip via the same `debugpy_available()` check, now on the correct (fixed) code path. No new product feature was added — this is a pure verification-and-regression-fix pass, the first of its kind in this project's own history (every §75.x pass before it was a feature increment, not a differently-shaped-environment re-verification).

### 75.13 First Real Live-GUI Verification of `spartan-editor-core` on Linux — Software Rendering, a Real X11 Keyboard-Focus Finding, and a Second Real Cross-Platform Build Bug

§75.12 established this Linux container has no `/dev/dri` and concluded nothing GUI/visual could be re-verified in it. That conclusion was correct for a *hardware* GPU, but incomplete: this pass found and installed a real, working *software* Vulkan device (Mesa's `llvmpipe`/lavapipe), which is enough to actually launch and drive `spartan-editor-core`'s real product binary end-to-end, live, for the first time outside a Windows/real-GPU machine.

**Three more real, environment-specific gaps found and fixed before the binary would even open a window.** `libxkbcommon-x11.so` (winit's X11 backend needs it for keyboard-layout handling) was missing, confirmed by a real startup panic naming the exact library, and fixed with `libxkbcommon-x11-0`. No Vulkan ICD existed at all (`/usr/share/vulkan/icd.d` didn't exist), so `wgpu::Instance::new()` had no adapter to find; installing `mesa-vulkan-drivers` provided `lvp_icd.json` (lavapipe, a real, `vulkaninfo`-confirmed `PHYSICAL_DEVICE_TYPE_CPU` device), which `spartan-editor-core`'s own existing `Backends::all()` adapter request picked up correctly with no code change needed. A display was needed at all — `Xvfb` provided one.

**A real, load-bearing X11 finding, confirmed by temporary debug instrumentation (added, used, then fully reverted — never committed).** With only `Xvfb` running and no window manager, the live binary printed a real `WindowEvent::Focused(false)` at startup and then never received a single `WindowEvent::KeyboardInput`, even though `xdotool`-driven mouse events (`CursorMoved`, `MouseInput`) arrived correctly and even after directly calling `XSetInputFocus` on the window (`xdotool windowfocus`). This is a real X11/ICCCM property, not a product bug: without a window manager to grant focus via the normal protocol, `winit`'s X11 backend's own focus tracking never flips to `true`, and X11 keyboard delivery is focus-gated (unlike pointer events). Installing and running a minimal window manager (`fluxbox`) fixed it immediately and for real — the next launch printed `Focused(true)` on window map, with no other change, and real `KeyboardInput` events (`Space`, `a`, `b`) then flowed correctly end-to-end through `input::handle_key_event`. The diagnostic `eprintln!`s used to establish this (`main.rs`, two call sites) were removed again with `git checkout` before committing anything — this section is the only record of them.

**A second real, previously-undetected cross-platform build bug, found by actually running `cargo build --release --workspace`, not `cargo test`.** `spikes/ui-shell-spike/src/main.rs` — §47.11's WebView2 shell, already known to be Windows-only in what it *does* — turned out to also be Windows-only in whether it *compiles*: an unguarded `use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus` (the exact focus-stealing-fix call §47.11 itself documents) has no real symbol to link against outside Windows, and `cargo build --release --workspace` failed with `undefined symbol: SetFocus`. This had never surfaced before, including in this same session's own earlier §75.12 pass, because `cargo test --workspace` only builds a *test* harness for a `[[bin]]` crate, which never actually reaches (and therefore never links) the specific closure that calls `SetFocus` — a real, concrete illustration of why this project's own rule is "run it," specifically the thing being claimed, not a nearby proxy for it. Fixed by gating the `windows`/`raw_window_handle` imports, the `win32_hwnd` helper, and the `SetFocus` call site itself behind `#[cfg(windows)]`, with no behavior change on Windows and a documented, honest no-op on other platforms (the underlying WebView2-specific focus bug was never claimed to be investigated, let alone fixed, on non-Windows backends). `cargo build --release --workspace` now succeeds cleanly on Linux; `cargo clippy --workspace --release --all-targets` and `cargo fmt --all -- --check` were both re-confirmed clean afterward, and the full `cargo test --workspace --release` suite re-confirmed green.

**Real, executed verification of `spartan-editor-core` itself, live, through the actual product binary.** Launched against a real file (`src/highlight.rs`, this crate's own tree-sitter module) under Xvfb + fluxbox + lavapipe: real language detection, a real `rust-analyzer` LSP session start, real DAP-ready/build-integration status, and real tree-sitter-rust syntax highlighting all printed and initialized correctly. A real, permanent cold-open breakdown printed 116–118ms across repeated runs on this *software*-rendering stack — closer to §39.1's <100ms target than every prior *hardware*-GPU number this project has recorded (§75.9's 467–715ms, §75.11's similar range), but explicitly not a comparable result: different rendering backend (CPU `llvmpipe` vs. a real Intel iGPU), different host CPU, no like-for-like basis, reported as a distinct data point rather than conflated with the GPU-hardware series. A screenshot confirmed real, correct rendering: comment lines rendered in a visually distinct color from code, consistent with real tree-sitter highlighting being active (matching §75.11's finding, not re-litigated in detail here). A second screenshot, taken after sending real `Space`/`a`/`b` keypresses through the X server with the fluxbox fix in place, confirmed the literal characters `ab` genuinely inserted at the very start of the document with the cursor rendered immediately after them — the first live, keyboard-driven, end-to-end edit verification of this crate's real product binary outside a Windows machine.

**What this does not confirm.** Cold-open, edit-latency, and scroll-latency were not benchmarked in this environment beyond the one-shot cold-open print above (no repeated-run `--synthetic:` benchmark pass here, unlike §75.9/§75.11's own dedicated latency investigations) — this pass is a functional live-GUI verification, not a performance one, and doesn't claim to be. `Backspace`/`Enter`/arrow-key/scroll input were not separately exercised live in this pass (only `Space`, `a`, `b`) — `input.rs`'s handling of those is unchanged and was already covered by this crate's existing headless tests, not newly re-verified live here. The `ui-shell-spike` fix only restores linkability and preserves existing (Windows-only) behavior; it does not add or verify any WebView-focus behavior on Linux — `wry`'s WebKitGTK backend on Linux may or may not have an analogous focus-stealing issue, genuinely unknown and unexplored by this pass. `Xvfb`/`fluxbox`/`mesa-vulkan-drivers` are machine-local tooling for this session's own verification, not committed to the repo or assumed present in a future session.

### 75.14 Real Mouse Input — Click-to-Position-Cursor (First Increment Toward Tier 1's UI Gap)

Starting a real push through §35.4's remaining Tier 1 v1 scope (UI shell, multi-file editing, Leo, Android, GUI Builder, and the rest — all still reference-only). Picked as the first increment because it's a hard dependency for everything else UI-facing: `spartan-editor-core` had zero mouse handling of any kind before this pass — no click-to-position, nothing.

**What was built.** `text::TextState::hit_test(x, y) -> Option<(local_line, col_chars)>` (`text.rs`), the real inverse of the existing `cursor_pixel_pos`, using cosmic-text's own `Buffer::hit` hit-testing API (confirmed against the actual installed `cosmic-text-0.10.0` source before use, same discipline as §75.11) and converting its byte-offset `Cursor::index` to this crate's char-based column convention. `viewport::to_doc_line` (`viewport.rs`) is the real inverse of `to_local_line`. `editor_view::EditorView::set_cursor_to_line_col` (`editor_view.rs`) is the real, clamped setter `cursor_line_col` never needed a counterpart for until now (clamps both an out-of-range line and a too-long column rather than trusting the caller). `main.rs` now tracks the last `WindowEvent::CursorMoved` position (winit reports position and click state as separate events) and, on `WindowEvent::MouseInput` left-press, chains all three through to move the real cursor.

**Real, executed verification.** `set_cursor_to_line_col`/`to_doc_line` are headlessly tested (7 new tests in `tests/viewport_and_language.rs`, including the clamp cases) — `hit_test` itself can't be (it needs a real `wgpu`-backed `TextState`, same GPU-dependency boundary as the rest of `text.rs`/`cursor.rs`). Live, through the actual product binary under Xvfb+fluxbox+lavapipe: two separate clicks at different screen coordinates each moved the caret to exactly the clicked character (confirmed by screenshot, not assumed), and a follow-up `Backspace` after a click deleted the character at the new cursor position, not the old one — real end-to-end proof the click path and the existing keyboard path share one real cursor, not two independent ones. Full `cargo test --workspace --release`, `clippy --all-targets`, and `fmt --check` all re-confirmed clean.

**What this does not confirm.** No text selection (click-and-drag, shift-click) — `EditorView` has no selection concept yet at all; scoped out as its own real increment rather than half-built here. No double-click-to-select-word or triple-click-to-select-line. No right-click/context menu. No scrollbar drag-to-scroll (the scrollbar visible in screenshots is cosmic-text/glyphon's own rendering, not yet wired to `Viewport::scroll_by`).

### 75.15 Real Multi-File Editing — Keyboard-Driven Switching, No Visual Chrome Yet

`spartan-editor-core` could only ever open exactly one file per process before this pass. This is the second increment of the real push through §35.4's remaining Tier 1 gaps, and a hard, real dependency for the file-tree/tab-bar UI (task #16), the Source Control panel, and Leo showing multi-file diffs — none of those can be dogfooded against a single-file editor.

**What was built.** `main.rs`'s flat single-file locals (`editor`, `viewport`, `highlighter`, `lsp_session`, `breakpoints`, `dap_launch_info`, `dap_build_info`) became a real `OpenFile` struct and a `Vec<OpenFile>` indexed by `active: usize`. `TextState` (the GPU-backed cosmic-text buffer/atlas) deliberately stays a *single shared instance*, not duplicated per file — switching files reshapes the same `TextState` against the newly active file's own state, avoiding N× GPU atlas memory for N open files. A new `open_file()` function does per-file setup (language detection, its own real LSP session spawn, DAP info capture, highlighter construction) — real, named cost: each LSP-capable open file spawns its *own* `rust-analyzer` process, since `LspSession` (§75.6) is hardcoded to one file's `didOpen`/`didChange` lifecycle; multiplexing multiple files through one shared per-project session is real, separate future work, not attempted here. Additional files open via repeated `--open:<path>` CLI arguments (no file-tree/dialog exists yet), switched with Ctrl+Tab / Ctrl+Shift+Tab (`WindowEvent::ModifiersChanged` tracked for the first time to detect held Ctrl/Shift). DAP breakpoints moved from a single flat `Vec<i64>` into per-`OpenFile` storage, fixing a real, previously-latent bug named back in §75.8: a bare line number has no file association, so it was silently ambiguous the moment more than one file could be open. `dap_session`/`pending_build` deliberately stay global, not per-file (one running debug program, not tied to whichever file happens to be in view); F5 always targets the *active* file's own captured DAP info and breakpoints — a real, named scope limit, not a silent gap, since the underlying DAP client only issues one `setBreakpoints` call for one source file.

**A real bug found only by actually closing the live app, not by inspection.** The first implementation shut down every file's LSP session by `Vec::drain`-ing `files` entirely on `WindowEvent::CloseRequested`. `elwt.exit()` doesn't take effect immediately — this crate's `ControlFlow::Poll` means winit still delivers at least one more event afterward (typically another `RedrawRequested`), and every other handler unconditionally indexes `files[active]`. With `files` drained to empty, that next event panicked with a real, reproducible out-of-bounds index — caught by actually pressing the window's close button through the live binary, not visible from reading the code. Fixed by `Option::take()`-ing each file's `lsp_session` in place instead of draining the `Vec` itself: identical shutdown behavior, but `files[active]` stays a valid index for whatever winit delivers before the process actually exits.

**Real, executed verification.** Full `cargo test --workspace --release`, `clippy --all-targets`, and `fmt --check` all clean. Live, through the actual product binary under Xvfb+fluxbox+lavapipe, opened with two real files (`main.rs`, `highlight.rs`): a screenshot confirmed the first file's real content and highlighting; Ctrl+Tab switched to the second (confirmed via both the printed "Switched to" log line and a second screenshot showing genuinely different, correct content); typing `XYZ` into the first file, switching away, and switching back (confirmed via a third screenshot) proved edits are real per-file state, not shared — the second file's content was unaffected by the first file's edit, and the first file's edit persisted correctly across the round trip. The close-path bug above was found, fixed, and re-verified live (clean shutdown, both "Shutting down language server..." lines printed, no panic) as part of this same pass. The 50k-line `--synthetic:` benchmark was re-run and completed correctly (confirming the benchmark harness's `files[0]`-only assumption still holds after the refactor) — its absolute numbers on this software-rendering environment are a new baseline, not compared against any prior hardware-GPU figure, consistent with §75.13's established caveat.

**What this does not confirm.** No visual file-tree or tab bar (task #16) — file switching is keyboard-only, with no on-screen indication of which files are open besides the printed log line. No shared/multiplexed LSP session across files in the same project (named above). No unified multi-file breakpoint set for a single debug session (named above). No "open file" dialog — CLI args only. No unsaved-changes indicator or save/close-single-file UI (there is no save at all yet — no file in this crate has ever been written back to disk, an existing, unrelated gap this pass doesn't touch).

### 75.16 Real Save-to-Disk (Ctrl+S) — the First Save Functionality Anywhere in This Crate

§75.15 named it in passing while building multi-file editing: no file opened by `spartan-editor-core`, across every pass since §75.5, had ever been written back to disk. This pass closes that gap for real.

**What was built.** `OpenFile` gained a real `dirty: bool`, set the first time any edit (`EditEffect != None`) lands on a file and cleared on a successful save. `Ctrl+S` is matched via `key_event.physical_key == PhysicalKey::Code(KeyCode::KeyS)` combined with `modifiers.control_key()` — deliberately *not* matched against `logical_key`, since some platforms report no `text` at all for a Ctrl-held letter and `physical_key` is layout/modifier-independent, avoiding a repeat of relying on a field that isn't guaranteed to carry the information needed. On a real file (`label` not starting with `--synthetic:`), it runs a real `std::fs::write(&label, editor.text())`; a `--synthetic:` fixture (no real path) prints a clear, real refusal instead of attempting a nonsensical write. Since no visual chrome (file tree, tab bar, status bar) exists anywhere in this crate yet, the window title itself became the first real, user-visible unsaved-changes signal: `window_title()` appends a literal `*` while `dirty`, updated on every edit, file switch, and successful save via `window.set_title()`.

**Real, executed verification.** Full `cargo test --workspace --release`, `clippy --all-targets`, and `fmt --check` all clean (no headless tests were added — this feature is pure OS/IO-facing glue with no pure logic to isolate, matching `text.rs`/`cursor.rs`'s existing live-only precedent). Live, through the actual product binary, against a real scratch file (not a tracked repository file, to avoid corrupting real source during testing): typed `// EDITED` into a fresh copy of a small Rust file, confirmed via `xdotool getwindowname` (not just a screenshot, since the real title string was wider than the visible titlebar) that the title gained a trailing ` *`; pressed Ctrl+S, confirmed via a second `getwindowname` call that the `*` was gone, confirmed the real `"Saved: <path>"` line printed, and confirmed by reading the file directly off disk that its actual bytes now contained the typed edit, verbatim. Separately confirmed the `--synthetic:` guard: pressing Ctrl+S against a synthetic fixture printed the real refusal message and did not attempt a write (there is no path to write to in the first place).

**What this does not confirm.** No "unsaved changes" prompt on close or on switching away from a dirty file — `CloseRequested` and Ctrl+Tab both discard in-memory changes silently if unsaved, a real, named gap (data loss risk) rather than a hidden one. No Ctrl+Shift+S / save-as. No check for the file having changed on disk since it was opened (a concurrent external edit is silently overwritten — no conflict detection at all). No auto-save. No per-file save keybinding scoped correctly when multiple files are open and one isn't the active one (only the active file is ever saveable, matching how editing itself is already scoped).

### 75.17 Real Arrow-Key Cursor Navigation

Found while scoping text selection (task #15, which needs Shift+Arrow to extend a selection): no arrow-key handling existed anywhere in `spartan-editor-core`, in any prior pass — the cursor could only move via a mouse click (§75.14) or as a side effect of inserting/deleting text. This pass closes that more fundamental gap first.

**What was built.** `EditorView` gained four real movement methods: `move_left`/`move_right` (one char, clamped at document start/end) and `move_up`/`move_down` (one line, reusing `set_cursor_to_line_col`'s existing clamp so landing on a shorter line lands at its end rather than out of bounds). All four return whether the cursor actually moved, matching `Viewport::scroll_by`'s existing "did anything change" convention, so `main.rs` can skip a redundant redraw at a boundary. `main.rs` wires `ArrowLeft`/`ArrowRight`/`ArrowUp`/`ArrowDown` to these, following the same `ensure_visible` + conditional reshape pattern every other cursor-moving key already uses. A real, named, deliberate simplification: `move_up`/`move_down` don't remember a "desired column" across a run of moves through lines of different lengths, the way most real editors do — each individual move still lands somewhere valid, it just re-derives the column from the current (possibly already-clamped) position. Shift+Arrow is not handled as selection-extension yet — Shift is simply ignored for now, since `EditorView` has no selection concept at all (task #15 remains blocked on this, now correctly, on selection itself rather than on arrow movement not existing).

**Real, executed verification.** 8 new headless tests (`tests/viewport_and_language.rs`) cover single-step movement, both boundary no-op cases per direction, line-boundary crossing on `move_right`, and both the column-preserving and column-clamping cases for `move_up`/`move_down` — all passed on the first run. One test-writing mistake caught before running, not after: an early version of the "move down at last line is a no-op" test assumed line 0 was the document's last line for a single-line fixture, without accounting for `Document`'s own documented phantom trailing line on text ending in `\n` (the same real ropey/cosmic-text mismatch named repeatedly since §75.5) — fixed by deriving the real last line from `document.len_lines() - 1` instead of assuming. Live, through the actual product binary: 3× Down + 5× Right from the document start landed the cursor exactly on line 3, column 5 (confirmed by screenshot, matching the real, distinguishable text at that position), then Up + Left correctly landed on line 2, column 4. Full `cargo test --workspace --release`, `clippy --all-targets`, and `fmt --check` all clean.

**What this does not confirm.** No Shift+Arrow selection (task #15's actual scope, now unblocked). No Ctrl+Arrow (word-boundary jumps), no Home/End, no Ctrl+Home/End (document start/end). No sticky column across multi-line up/down runs (named above). No selection to clear when a plain arrow press is made while a selection exists, since no selection exists yet to clear.

### 75.18 Real Text Selection — Click-Drag, Shift-Click, Shift-Arrow, Type-Over-Replace

Closes task #15, unblocked by §75.17's arrow-key work. `EditorView` had no selection concept at all before this pass.

**What was built.** `EditorView` gained a real `selection_anchor: Option<usize>` and `selection_range()` (normalized `[start, end)`, `None` when the anchor and cursor coincide — a click with no drag isn't a real selection). `insert_at_cursor`/`backspace` now check for an active selection first: typing over one deletes it and inserts in its place (real "replace" behavior, always reported as `Structural` — a deliberate simplification over reasoning through every combination of the delete's and insert's own individual effects); Backspace with a selection deletes exactly the selection, not the selection plus one more character before it. `viewport::selection_line_spans` is new, pure, headlessly-testable logic: given a normalized selection range, it returns one `(doc_line, start_col, end_col)` per line touched, with `end_col: None` meaning "through this line's actual end" for every line except the one containing the selection's end. `main.rs` turns those spans into real pixel rects via the same `TextState::cursor_pixel_pos` lookup the caret itself already uses (an out-of-range column for the `None` case leans on that method's own existing end-of-line fallback rather than needing a second lookup), rendered by a new `SelectionRenderer` (`selection.rs`/`selection.wgsl`) — deliberately its own type rather than generalizing `CursorRenderer` to "N quads," since selection needs a *variable* count of semi-transparent quads rendered *before* the glyph pass (so text stays legible on top), the opposite ordering from the single opaque caret quad rendered *after* it.

**Interaction wiring, matching conventional editor behavior.** Mouse: press arms a fresh anchor at the click point (or extends from the existing anchor/cursor if Shift is held); `CursorMoved` while the button stays down extends the selection to the drag position. Keyboard: a new `handle_arrow_key` helper (replacing §75.17's simpler version) makes Shift+Arrow extend the selection (arming one at the cursor first if needed), while a *plain* arrow press with a selection already active collapses it instead of moving further — Left/Right jump straight to the selection's start/end, Up/Down clear the selection and still move from the (now-collapsed) cursor, since there's no single correct vertical collapse target the way there is a horizontal one. `Escape` clears an active selection without moving the cursor, a small real addition made alongside this work.

**Real, executed verification.** 9 new headless tests for `EditorView`'s selection methods and 4 for `selection_line_spans`, covering normalization, the anchor-preservation rule, delete/replace, and both a same-line and cross-line span. One real test-writing mistake caught by running it, not by inspection: an early `backspace`-with-selection test asserted the wrong resulting string, having miscounted which characters `"hello world"[2..7)` actually covers — fixed by recounting, not by changing the (correct) implementation. Live, through the actual product binary: a real click-drag produced a genuine multi-line highlighted selection (screenshotted); typing over it replaced the entire selected range with the typed text, confirmed by reading the resulting line content in a follow-up screenshot (also surfacing, incidentally, that Ctrl+Z currently inserts a literal "z" character rather than undoing anything, since no undo keybinding exists yet — named as its own follow-up, not fixed here); Shift+click extended a selection from a prior plain click (screenshotted); a plain Left afterward collapsed it to the selection start (screenshotted, cursor position confirmed); Shift+Down+Down+Right+Right+Right from there re-extended a fresh selection by keyboard alone (screenshotted); Escape cleared it, leaving the cursor in place (screenshotted). Full `cargo test --workspace --release`, `clippy --all-targets`, and `fmt --check` all clean; the 50k-line `--synthetic:` benchmark was re-run and shows no regression against this same software-rendering environment's own prior baseline (§75.15's run).

**What this does not confirm.** No copy/cut/paste (Ctrl+C/X/V) — no OS clipboard integration exists at all yet, a real, separate, complete feature of its own, not attempted here. No double-click-to-select-word or triple-click-to-select-line. No visible selection when it extends above/below the current viewport (rendering is necessarily limited to what `cursor_pixel_pos` can resolve within the windowed slice — consistent with every other viewport-scoped subsystem in this crate, not a new limitation). An empty selected line (a selection spanning a blank line) renders with zero width and is therefore invisible — a real, minor, named gap, not silently accepted as correct.

### 75.19 Real Undo/Redo (Ctrl+Z/Ctrl+Y) — Wiring a Capability That Already Existed, Unused

Found live while testing selection (§75.18): pressing Ctrl+Z inserted a literal `z` character instead of undoing anything. Investigating why led to a real, notable discovery: `spartan-buffer::Document` has had a complete, tested, real branching undo tree since §75.2 — `undo()` (move to parent checkpoint) and `jump_to_checkpoint()` (jump anywhere in the tree) — but nothing in `crates/spartan-editor-core`, across every pass since §75.5, had ever called either one. The capability existed; the wiring didn't.

**A real design question this pass had to resolve, not just plumb through.** `spartan-buffer`'s undo is a *branching tree*, not a linear stack — by its own design (§2.1), "redo" isn't a single well-defined operation the way it is for a linear undo stack, since jumping to any sibling checkpoint is just as valid a "redo" as returning to the one most recently undone away from. Rather than push that ambiguity down into `spartan-buffer` itself (which would mean picking one interpretation for every caller, forever), the conventional, expected "redo returns to what I just undid" behavior is built one layer up, in `EditorView`: a new `redo_stack: Vec<CheckpointId>`, pushed by `undo()` on success and popped by `redo()`, cleared by every real edit (matching every editor's actual behavior: a fresh edit after undo invalidates redo for that branch, even though `spartan-buffer`'s own tree never deletes the old checkpoint until it ages out of its bounded ring). `redo()` treats a checkpoint that *has* aged out since being pushed (`jump_to_checkpoint` returning an error) as "skip it, try the next one" rather than surfacing an error — a real, honest possibility of this crate's own bounded-ring design, not a hypothetical, and there's nothing a caller could usefully do about an already-evicted checkpoint anyway.

**What was built.** `EditorView::undo()`/`redo()`, both returning whether anything actually changed (matching every other movement method's convention), both clearing an active selection and clamping the cursor into the restored document's real bounds. `main.rs` wires `Ctrl+Z` to undo and *both* `Ctrl+Y` and `Ctrl+Shift+Z` to redo (the two common real-world conventions), matched via `physical_key` like Ctrl+S (§75.16) for the same robustness reason. Undo/redo always triggers a full `reshape_window`, never the cheap per-line fast path — unlike a single keystroke, jumping to an arbitrary checkpoint can change an unbounded amount of content at once.

**Real, executed verification.** 7 new headless tests, all passing on the first run: revert-one-edit, no-op at the tree root, redo-restores-what-was-undone, no-op with nothing undone, a fresh edit correctly clearing the redo stack (confirmed the *"heo"* branch and not spartan-buffer's own still-alive tree node), selection getting cleared by undo, and walking back through three edits in sequence. Live, through the actual product binary: typed `TYPED_TEXT` (10 real keystrokes, confirmed via screenshot, title correctly showing the dirty marker); a single Ctrl+Z removed exactly one character (`TYPED_TEX`, screenshotted) — real, correct confirmation that this crate commits one checkpoint per keystroke, not one per "logical" edit; 9 more Ctrl+Z presses fully restored the original file content (screenshotted); 10 Ctrl+Y presses then fully restored `TYPED_TEXT` again, exactly (screenshotted). Full `cargo test --workspace --release`, `clippy --all-targets`, and `fmt --check` clean; the 50k-line `--synthetic:` benchmark re-run with no regression against this environment's own established baseline.

**What this does not confirm.** No undo/redo UI (a history panel, hover-to-preview) — keyboard only, matching every other keybinding in this crate. No coalescing of consecutive same-kind edits into one undo step (typing ten characters takes ten Ctrl+Z presses to fully undo, confirmed above as real, current behavior, not a bug) — real editors typically group a run of typing into one undo unit; this crate does not, yet. No undo/redo interaction with LSP/DAP state (an undone edit doesn't explicitly re-notify a live language server outside the normal debounced `notify_edit` path that any other edit already goes through — not separately verified here).

### 75.20 Real Clipboard Integration (Ctrl+C/X/V) — and a Real, Live Selection Bug Found Only by Testing the Combination

Closes task #22, deferred out of §75.18 to keep that pass complete rather than half-finished. Real OS clipboard access via a new `arboard` dependency; real substring extraction added to `spartan-buffer` itself, since no accessor for "the text within a char range" existed before this pass (only whole-document `text()` and per-line `line()`).

**What was built.** `Document::text_between(range)` (`spartan-buffer`) is a thin wrapper over `Rope::slice(...).to_string()`, real and minimal. `EditorView::selected_text()` uses it against the active selection. `main.rs` constructs one `arboard::Clipboard` at startup (construction can genuinely fail — no reachable clipboard manager — handled by printing once and disabling Ctrl+C/X/V for the rest of the session, matching this crate's existing LSP/DAP-spawn-failure pattern rather than treating it as fatal). Ctrl+C copies the active selection's real text to the OS clipboard; Ctrl+X does the same and then deletes it; Ctrl+V inserts the clipboard's real text at the cursor, transparently replacing an active selection via `insert_at_cursor`'s existing §75.18 behavior. All three matched via `physical_key`, consistent with Ctrl+S/Z/Y.

**A real, live bug found only by testing paste-after-a-plain-click, not by inspection, and fixed in this same pass.** §75.18's mouse-press handler armed `EditorView::selection_anchor` unconditionally on *every* plain click, reasoning "so a subsequent drag has something to extend from." That reasoning was itself correct, but `selection_range()`'s definition of "an active selection" is exactly "anchor and cursor differ" — so *any* later cursor movement by *any* means (typing, pasting, undo/redo, not just an actual mouse drag) silently became a real, visible selection, because the stale anchor from the last click was still sitting there. Live testing this section's own new paste feature caught it immediately: click at one point, paste elsewhere, and the pasted text itself rendered as if newly selected. No prior pass's live testing exercised this exact sequence (§75.18's own verification always drag-selected, shift-clicked, or used keyboard-only interactions — never "plain click, then something else"), which is exactly why it went uncaught until a genuinely new interaction combination was tried. **Fixed** by no longer arming `selection_anchor` on press at all — a new `drag_anchor_pos: Option<usize>` (main-loop-local, not part of `EditorView`'s own state) remembers only *where* the button was pressed, and `CursorMoved` arms the real `selection_anchor` lazily, the first time the cursor is observed to have actually moved away from that position during a genuine drag.

**Real, executed verification.** 5 new tests: 3 for `Document::text_between` (extraction, out-of-bounds, inverted range — spartan-buffer's own suite, now 22 tests), 2 for `EditorView::selected_text`. Full `cargo test --workspace --release`, `clippy --all-targets`, `fmt --check` all clean, both before and after the bug fix. Live, through the actual product binary: a real drag-selected range, copied, and pasted at a different clicked location reproduced the bug exactly as described (screenshotted, showing the pasted text incorrectly highlighted); after the fix, the identical sequence was re-run and the pasted text showed no stray highlight (screenshotted); a separate real cut-then-paste-back round trip was also verified end-to-end (screenshotted at each step: selected, cut — text gone, no stray highlight — pasted back, content restored exactly). The 50k-line `--synthetic:` benchmark was re-run after the fix and shows no regression against this environment's established baseline.

**What this does not confirm.** No system clipboard *format* beyond plain text (no rich text, no image paste, though `arboard`'s own default features do support the latter — not exercised here). No middle-click/X11-PRIMARY-selection paste (a Linux-specific convention distinct from Ctrl+C/V's CLIPBOARD selection — not implemented). No clipboard history. Paste of a very large clipboard payload was not specifically load-tested.

### 75.21 Real Visual Tab Bar — Click to Switch, Click to Close

Closes the "visual tab bar" half of task #16 (the file-tree-sidebar half split out to task #24 — genuinely different scope: filesystem traversal and a new open-file interaction, not blocking this one). Every increment since §75.15 built real multi-file *state*; this pass makes it visible and clickable for the first time — before this, the only way to tell which files were open was a printed log line, and switching was keyboard-only.

**A real API discovery shaped the design before any code was written.** Reading `glyphon-0.5.0`'s actual installed `TextRenderer::prepare()` source (not assumed from docs) showed it already accepts `impl IntoIterator<Item = TextArea>` — *multiple* text areas in one `prepare`/`render` call, sharing one `FontSystem`/`TextAtlas`/`SwashCache`. That meant the tab bar didn't need a second, parallel glyphon pipeline: `TextState` gained a second, independent `tab_bar_buffer: Buffer`, prepared and rendered alongside the existing main-editor `buffer` in the same calls. `TAB_BAR_HEIGHT: f32 = 28.0` was added and `TEXT_ORIGIN_Y` redefined as `8.0 + TAB_BAR_HEIGHT` — every existing call site (`visible_lines`, cursor position, hit-testing, selection rects) already used the symbolic constant, so the whole main editor shifted down to make room for the tab bar with zero other call sites needing to change.

**Real hit-testing, not pixel-guessing.** `tab_bar.rs` (new, pure, no GPU dependency, headlessly tested) builds the tab bar's one-line display string (` name[ *] × ` per tab, `│`-separated) and records each tab's real char-range within it, plus the narrower char-range of its `×` close button. A click resolves via the *same* real cosmic-text `Buffer::hit` technique `TextState::hit_test` already uses for the main editor (a new `hit_test_tab_bar`/`tab_bar_pixel_pos` pair on `TextState`, mirroring `hit_test`/`cursor_pixel_pos`) — not a fixed-pixel-width geometry model that would need to assume a monospace character's exact rendered width. The active tab's highlight rect reuses `SelectionRenderer` verbatim as a second instance (`tab_bar_renderer`) rather than a new rendering type — both are "draw semi-transparent accent-colored rects," and sharing the color between "this text is selected" and "this is the active tab" is a deliberate, coherent choice.

**Real close-tab, with a real, deliberate guard.** A new `close_file()` removes the file, shuts down its LSP session, and adjusts `active` to stay valid and sensible (unaffected if it pointed before the closed index, shifted left by one if after, or left in place — now referring to what used to be the next file — if it *was* the closed index, unless that was the last file, clamped to the new last one). Refuses to close the last remaining open file (this crate has no "empty editor" state to fall back to anywhere) rather than attempting to model one under time pressure. Closing (like every other place a file's state can currently go away — `CloseRequested`, Ctrl+Tab) silently discards unsaved changes, an existing named gap (task #18), not a new one introduced here.

**Real, executed verification.** 8 new headless tests for `tab_bar.rs` (single/multi-tab layout, the dirty marker, close-range containment, both hit-test outcomes, empty input), all passing on the first run. Live, through the actual product binary, opened with three real files: a screenshot confirmed the rendered tab bar (`main.rs × │ highlight.rs × │ tab_bar.rs ×`) with the first tab's real highlight rect visible; clicking the second tab's label switched the active file (screenshotted: highlight rect moved, title updated, content changed to the real second file); clicking the third tab's `×` closed it (screenshotted: tab gone, `"Shutting down language server..."` printed, the previously-active second tab stayed active and unaffected); closing the second tab down to one remaining file worked the same way; attempting to close that last tab printed `"Cannot close the last open file"` and left it untouched (screenshotted). A separate click inside the main editor area re-confirmed no regression from the `TEXT_ORIGIN_Y` shift (cursor landed exactly on the clicked character). A real `clippy::reversed_empty_ranges` `deny`-level lint surfaced during this pass's own verification, on a test written in §75.20 (`Document::text_between`'s inverted-range test) that hadn't actually been re-linted since — fixed by matching this same file's own pre-existing precedent (`inverted_range_returns_error_not_panic`) of building the range from variables rather than a literal, the same real fix already established, just not consistently applied. Full `cargo test --workspace --release`, `clippy --all-targets`, `fmt --check` clean; the 50k-line `--synthetic:` benchmark re-run with no regression.

**What this does not confirm.** No file tree sidebar (task #24) — files can still only be *opened* via `--open:<path>` CLI args; the tab bar only visualizes files already open. No tab reordering (drag-to-reorder). No tab overflow handling — with enough open files the tab bar text could in principle exceed the window width; not tested at that scale. No keyboard shortcut to close the active tab (Ctrl+W or similar) — close is mouse-only for now. No unsaved-changes confirmation on close (named above, tracked separately).

### 75.22 Real Home/End/Ctrl+Arrow Navigation, and Sticky Column

Closes the exact three gaps §75.17 named explicitly when it shipped plain arrow-key navigation: no Home/End, no Ctrl+Left/Right word jumps, no Ctrl+Home/End document jumps, and no "sticky column" memory across an up/down run through lines of different lengths.

**Six new `EditorView` methods**, all following the crate's existing "return `bool` for did-anything-change" convention (`move_left`/`move_right`/`move_up`/`move_down`'s own pattern): `move_to_line_start`/`move_to_line_end` (Home/End, reusing a newly-factored-out `line_len_chars` helper that `set_cursor_to_line_col` also now calls, rather than keeping the same terminator-stripping calculation duplicated in three places), `move_to_document_start`/`move_to_document_end` (Ctrl+Home/Ctrl+End), and `move_word_left`/`move_word_right` (Ctrl+Left/Right).

**Word jump has no cheap single-char accessor to build on.** `spartan-buffer::Document` exposes `text_between(range)` (a rope slice + `to_string`) but nothing narrower. Rather than add a new `Document` API for this, `editor_view.rs` gained two private helpers, `char_before`/`char_at`, that each fetch exactly one char via `text_between(pos..pos+1)`. This is O(log n) per call, but a word jump only ever calls it a handful of times (one word's worth of characters, not the document), so the real cost stays bounded regardless of document size — deliberately not a whole-document `.text()` scan, which would have been O(n) per keystroke on a large file. The jump logic itself: skip any whitespace (including `\n`, so it crosses line boundaries for free) immediately adjacent to the cursor, then consume the contiguous run of "same kind" chars after that, where "kind" is a binary word-char (alphanumeric/`_`) vs. punctuation split — so `foo.bar` treats `.` as its own single-char token, not lumped in with either identifier, matching how most real editors stop at a `.` rather than jumping straight from `foo` to `bar`.

**Sticky column stores the *desired* column, not the clamped one.** A new `EditorView::sticky_column: Option<usize>` field is set by `move_up`/`move_down` to the column they were *asked* to reach (before `set_cursor_to_line_col` clamps it to whatever a short intermediate line allows) and reused, unmodified, by the next call in the same run — which is what lets a run survive several short lines in a row and still land back on the original column once a long-enough line is reached again, rather than the column silently decaying to whatever the shortest line in the run happened to clamp it to. Every other cursor-moving method (`move_left`/`move_right`, the four new jump methods, `set_cursor_to_line_col` itself — used by mouse clicks — `insert_at_cursor`, `delete_selection`, `backspace`, `undo`, `redo`) clears it, since a "run" is defined as strictly consecutive up/down calls.

**`handle_arrow_key` in `main.rs` was renamed to `handle_navigation_key`** and gained a `ctrl: bool` parameter; the outer `KeyboardInput` match arm now also matches `NamedKey::Home`/`NamedKey::End` alongside the four arrows, and passes `modifiers.control_key()` through. The existing §75.18 selection-collapse rule (plain Left/Right with an active selection jumps to the selection's start/end rather than moving further) is preserved unchanged and now explicitly `if !ctrl`-gated; every other combination (Up/Down, Home/End, Ctrl+Left/Right, Ctrl+Home/End) clears the selection and moves from the cursor, the same treatment §75.18 already gave Up/Down, extended to the new keys rather than inventing a separate rule for each.

**Two real test-writing mistakes caught by actually running the new tests, not by inspection** (matching this project's own stated discipline): first, `move_word_right_crosses_a_line_boundary` initially asserted landing at the *start* of the word after a line boundary; the real, already-established behavior (confirmed correct by the *other* passing word-jump tests) is to land at the *end* of the following token, so the assertion was wrong, not the implementation — fixed by correcting the expected value. Second, `sticky_column_resets_after_a_horizontal_move` assumed moving right on a blank line was a no-op ("nowhere to move right on an empty line"); the real behavior is that a blank line still has a real `\n` char to move over, so `move_right` from an empty line correctly crosses onto the next line's start — the test's premise was wrong, not the code, so the whole scenario was rebuilt around an unambiguous single-char line instead of a blank one.

**Real, executed verification.** 15 new headless tests in `tests/viewport_and_language.rs` (Home/End at both boundary and mid-line, Ctrl+Home/End, word jumps within a line, across a line boundary, at document boundaries, and three dedicated sticky-column scenarios), all passing; full `cargo test --workspace --release` (68 tests in this crate's own integration suite, 0 failures workspace-wide), `clippy --all-targets`, and `cargo fmt --check` clean. Live, through the actual binary under Xvfb+fluxbox: screenshots confirmed End/Home/Ctrl+End/Ctrl+Home all landing at the correct real position on a real 10-line fixture; six Ctrl+Right presses from before a `"` landed the cursor exactly between `foo` and `.bar` in `foo.bar baz qux` (confirming the punctuation-boundary behavior, not just whitespace-splitting), and one Ctrl+Left press from there retraced exactly back to the start of `foo`; a dedicated three-line fixture (`longer_line_here`/`x`/`another_longer_line`) confirmed sticky column live end-to-end — clicking mid-column on line 1, arrowing down onto the single-char line 2 (visibly clamped), then down again onto line 3 landed the cursor back at the same visual column as the original click, not column 1. The 50k-line `--synthetic:` benchmark was re-run afterward (500 random-position edits, 500 cursor-adjacent edits, 100 scrolls) and shows no regression (cold-open ~104ms, edit/cursor p99 ~3.5-4.0ms, scroll p99 ~5.8ms — all consistent with the prior committed baseline).

**What this does not confirm.** No word-jump-with-Shift-selection visual re-verification beyond the existing Shift+Arrow live check from §75.18 (the same `start_selection_if_needed` + `do_move` path is reused, but this pass's own live testing only screenshotted the plain, non-Shift case for the new keys). No sticky-column interaction with Shift+Up/Down selection specifically exercised live (only headlessly, via the same `move_up`/`move_down` calls Shift+Up/Down already routes through). Word classification is ASCII-oriented (`char::is_alphanumeric`/`is_whitespace`, which are real Unicode-aware `char` methods, not byte-based) but has not been tested against non-Latin scripts or combining characters. No word-jump-triggered undo/redo coalescing interaction beyond the existing `sticky_column`/`redo_stack` clearing already covers.

### 75.23 Real Unsaved-Changes Confirmation Modal — Closing Task #18

Closes task #18, a real, named gap left open since §75.15/§75.21: closing a dirty tab (mouse click on its `×`) or exiting the whole app while any file has unsaved changes previously discarded that content immediately, with no confirmation of any kind.

**Deliberately does not cover switching the active file.** Ctrl+Tab and clicking a different (non-`×`) tab were never actually a data-loss risk -- a file's `OpenFile`, and everything in its `editor.document`, stays exactly where it is in `files` when it's merely not the active one; only closing a tab or the whole process can discard content. The task's own description ("on close/switch") turned out to be broader than the real risk once traced through the code, so this pass scoped itself to the two places data can actually be lost, and says so rather than building a switch-time prompt that would only interrupt the user for no reason.

**A real generalization, not a new one-off renderer.** `SelectionRenderer`/`selection.wgsl` (§75.18, reused for the tab highlight in §75.21) had its quad color hardcoded in the fragment shader -- fine for two callers sharing one color, not for a third (the modal's dim overlay) wanting a different one. Color became a real per-vertex attribute instead (`SelectionVertex`/`SelectionRect` both gained a `color: [f32; 4]` field, `selection.wgsl` passes it through a vertex-output varying rather than returning a constant), and the two existing call sites were updated to pass the extracted `selection::ACCENT_HIGHLIGHT` constant explicitly. One generic quad renderer serving three real, differently-colored callers, not three near-duplicate pipelines.

**A third glyphon `TextArea`, same pattern as the tab bar.** `TextState` gained a `modal_buffer: Buffer` (§75.21 already established that `TextRenderer::prepare()` takes multiple `TextArea`s sharing one `FontSystem`/`TextAtlas`), positioned roughly vertically centered using the real current window height passed into `prepare()`. Empty text (the ordinary non-modal state) shapes to zero glyphs, so no separate on/off flag is needed -- the tab bar already established this exact pattern.

**Keyboard-only confirm/cancel, a real, named v1 scope decision.** A new `PendingClose` enum (`File(usize)` or `App`) and `Option<PendingClose>` state variable drive everything: a dedicated `KeyboardInput` match arm, inserted *before* every other keyboard-handling arm (Rust match arms are tried in order, so its `pending_close.is_some()` guard intercepts all input first), handles Enter (confirm) and Escape (cancel) and silently swallows everything else -- no clickable Yes/No buttons, no button hit-testing, matching how this crate has repeatedly shipped a real, working keyboard-first v1 before a mouse-driven refinement in prior passes. Both mouse-press arms (tab bar clicks, main editor clicks) are additionally gated on `pending_close.is_none()`, so a click during the modal falls through to the match's final wildcard arm and does nothing, rather than leaking a cursor move or a second tab action underneath the dim overlay.

**A real edge case found and fixed before it could ship as a bug, not after:** the tab-close click handler's new dirty check (`is_close && files[file_index].dirty`) initially had no length guard, so closing a dirty *sole remaining* tab would raise the modal, and pressing Enter would call `close_file`, which would then silently refuse (its own pre-existing "can't close the last open file" guard) -- the modal would have promised a close that could never actually happen. Fixed by adding `&& files.len() > 1` to the same condition, mirroring `close_file`'s own guard, so a dirty last tab's `×` behaves exactly as it already did before this pass (an immediate, harmless no-op) rather than raising a pointless confirmation.

**Real, executed verification, including a real environment-debugging detour.** Live testing initially hit two real, separate problems, both eventually traced to test-harness mistakes rather than product bugs: (1) an `--open:<path>` CLI arg passed as the crate's 2nd positional argument landed in the numeric-only `bench_edit_iters` slot instead (silently parsed-and-discarded, `extra_files` only reads from argv index 6 onward) -- fixed by padding the three benchmark-arg positions with empty strings; (2) keyboard input stopped reaching one specific long-lived window instance after extended interactive debugging (mouse clicks kept working throughout), which did not recur on a freshly spawned instance -- consistent with, though not conclusively identified as, the kind of X11/WM focus-delivery fragility §75.13 already documented as a real, environment-specific finding, not a product defect (no code in this crate's own focus/input handling changed in this pass). With both resolved: a real dirty tab's `×` raised the modal (screenshotted: dim overlay visibly darkens the background, correct file name and instructions rendered); Escape cancelled it (screenshotted: dim overlay gone, file still open and dirty, and a follow-up keystroke confirmed typing works normally again immediately after); re-triggering and pressing Enter really closed the tab (screenshotted: tab gone, no crash); dirtying the one remaining file and clicking the real OS window-decoration close button raised the real `CloseRequested`-driven App modal (screenshotted: "N file(s) have unsaved changes," correct Enter/Escape instructions); pressing Enter there really exited the process (confirmed via `ps` -- no process left) with both LSP sessions shut down and the final latency report printed, matching `CloseRequested`'s pre-existing shutdown sequence exactly. A dedicated test confirmed mouse clicks and keystrokes are both fully swallowed while the modal is up: clicking the main editor and typing a distinct marker string while the App-modal was showing left the underlying dirty content completely unchanged in a follow-up screenshot. Full `cargo test --workspace --release`, `clippy --all-targets`, `cargo fmt --check` clean (no new headless tests were needed -- this feature is entirely GPU/rendering/input-facing, the same category §75.14/§75.21's own rendering work fell into). The 50k-line `--synthetic:` benchmark was re-run afterward and shows no regression (cold-open ~114ms, edit/cursor p99 ~3.3-3.9ms, scroll p99 ~5.3ms, consistent with the prior baseline).

**What this does not confirm.** No confirmation prompt anywhere else content could theoretically be lost in the future (there currently isn't one -- "open a different file over an existing tab" doesn't exist as a feature yet). No "Save" option in the modal itself (only discard-and-close or cancel) -- a real, deliberate v1 scope cut, since wiring the modal's confirm path through the existing Ctrl+S save logic for an unnamed `--synthetic:` fixture (which `main.rs` already treats as unsaveable) would need its own decision about what "save" even means there, not attempted in this pass. No keyboard focus trap beyond swallowing input -- if a future feature added a second top-level window, nothing here would stop input from reaching it while this window's modal is up. No automated (non-visual) test exercises the actual rendered dim overlay or modal text pixel content, since this crate's established testing pattern for GPU-facing rendering has always been live screenshot verification, not pixel-diffing.

### 75.24 Real Ctrl+W Tab Close — the Keyboard-Only Portion of Task #25

Closes the Ctrl+W part of task #25 ("Ctrl+W close, overflow handling, reorder"), which this pass splits apart deliberately: Ctrl+W is a small, well-scoped keyboard-parity gap directly reusing machinery §75.23 just built, while tab overflow handling and drag-to-reorder are separate, larger UI features with no shared implementation -- the same kind of split §75.21 already made for the file-tree-sidebar half of task #16 (spun out as task #24 rather than bundled in). Overflow handling and reorder remain open, tracked under task #25's own remaining scope.

**No new interaction rule -- reuses §75.23's exactly.** The new Ctrl+W `KeyboardInput` arm (matched via `physical_key`, the same layout-independent convention every other Ctrl+<letter> shortcut in this crate already uses) applies the identical two-part rule the tab bar's own `×` click handler established in §75.23: if the active file is dirty *and* more than one file is open, raise the unsaved-changes modal (`PendingClose::File(active)`) instead of closing immediately; otherwise call `close_file` directly, which has its own pre-existing guard against closing the last remaining tab. No new logic was written for either the modal-raising decision or the last-tab guard -- both already existed and are simply invoked from a second call site.

**A real, small borrow-checker catch, not a design issue.** Passing `active` to `close_file` both as the `&mut usize` parameter it updates and as the `usize` index of the file to close (`close_file(&mut files, &mut active, active)`) doesn't compile -- evaluating `active` by value for the third argument while `&mut active` is already borrowed for the second is a real aliasing conflict the compiler catches immediately, not a runtime bug. Fixed by copying `active` into a local `closing` binding first, evaluated before the mutable borrow exists.

**Real, executed verification.** No new headless tests (this is a thin, input-facing wrapper around already-tested `close_file`/modal logic, the same reasoning §75.23 itself gave for not adding any). `cargo build`/`clippy --all-targets`/`fmt --check` clean. Live, through the actual binary with two real files open: Ctrl+W on a clean (non-dirty) tab closed it immediately, switching to the remaining tab (screenshotted); Ctrl+W on the resulting sole remaining tab printed the pre-existing `"Cannot close the last open file"` guard message and left it open (screenshotted, no modal); dirtying that same sole tab and pressing Ctrl+W again produced the identical no-op with the identical guard message, rather than incorrectly raising a modal that would have promised a close `close_file` could never actually perform (the exact edge case §75.23 already fixed for the mouse path, confirmed here to also hold for the keyboard path since both routes share the same `files.len() > 1` condition). Full `cargo test --workspace --release` (0 failures) and the 50k-line `--synthetic:` benchmark re-run afterward show no regression (cold-open ~106ms, edit/cursor p99 in the normal 3.7-5.1ms run-to-run range, scroll p99 ~5.3ms).

**What this does not confirm.** Tab overflow handling (many open tabs exceeding the window's width) is entirely unimplemented -- `tab_bar::build_tab_bar_text` still lays out every open file's tab unconditionally, with no scrolling, truncation, or overflow indicator, and has not been tested with enough files open to exceed the window width. Drag-to-reorder tabs is entirely unimplemented. Ctrl+W was not tested against a `--synthetic:` fixture (uncloseable by design, no real path) -- only against real files, which is the only case where `close_file`'s save-path logic is even reachable.

### 75.25 Real Undo Coalescing — Task #23

Closes task #23. Before this pass, `spartan-buffer::Document::insert` created one real checkpoint per call, and `EditorView::insert_at_cursor` was called once per keystroke -- so undoing a five-character word required five separate `Ctrl+Z` presses, one character at a time, never how a real editor behaves.

**Coalescing lives at the `EditorView` layer, not `spartan-buffer`'s.** `Document`'s "every edit is a real checkpoint" contract is a deliberate, already-tested, load-bearing invariant elsewhere in this crate (its own eviction/ring-capacity tests rely on exactly this granularity) -- changing it there would be invasive and out of scope. Instead, `EditorView` gained a new `typing_run: Option<(start_cursor, checkpoints_since_start)>` field, following the exact precedent §75.22's `sticky_column` already set for "a run continues across consecutive calls of the same kind, and is reset by literally everything else": `insert_at_cursor` extends the run (or starts a fresh one) on a plain, no-selection insert; every other cursor-affecting method in this file (`move_left`/`right`/`up`/`down`, the Home/End/word/document jump methods, `set_cursor_to_line_col` -- used by mouse clicks -- `delete_selection`, `backspace`, `undo`, `redo`) resets it to `None`, the same set of methods that already reset `sticky_column`.

**`undo()` now loops, not just steps once.** When a run is in progress, `undo()` calls `Document::undo()` up to `checkpoints_since_start` times in one call, coalescing the entire run into a single user-visible undo. It stops early (without panicking or over-undoing) if the loop can't complete every step -- a real, honest possibility since `Document`'s own bounded checkpoint ring (§2.1, default 500) can still evict part of a long-enough run -- and in that case falls back to the same clamp-based cursor positioning §75.19 already used, rather than claiming a precise "start of run" position it never actually reached.

**A real, small correctness improvement fell out of this for free, not scope creep:** restoring the cursor precisely. §75.19's original `undo()`/`redo()` only ever clamped the existing cursor into the new document's bounds, which happens to look right for an edit at the very end of a document but silently lands in the wrong place for one in the middle (undoing a mid-document 3-character insert previously left the cursor 3 positions too far right, clamped-but-not-restored). Since coalescing already needed to know "where was the cursor before this run started," `redo_stack` was extended from `Vec<CheckpointId>` to `Vec<(CheckpointId, usize)>`, storing the pre-undo cursor alongside each checkpoint -- so both a coalesced *and* an ordinary single-step undo now restore the cursor to its exact pre-edit position, and `redo()` restores it to its exact pre-undo position, not just a clamp. This fixes a real, if minor, pre-existing imprecision as a direct consequence of the data coalescing needed anyway, not a separate initiative.

**A real, deliberate scope cut, named rather than silently absorbed: backspace runs do not coalesce.** The task description says "group consecutive typing," and deleting a word via repeated Backspace is arguably a different (if related) real gap -- attempting both in one pass would need a second, direction-aware run concept (forward insertion vs. backward deletion can't share the same `start_cursor` restoration logic unchanged). `backspace()` explicitly resets `typing_run` to `None`, ending any insertion run in progress, exactly like every other non-insert method.

**Real, executed verification.** One pre-existing test (`multiple_undo_calls_walk_back_through_several_edits`) encoded the *old*, now-intentionally-changed behavior (three adjacent inserts each undoing separately) -- not a bug this pass introduced, but a real test whose premise coalescing was specifically built to invalidate; renamed and rewritten to insert cursor moves between each edit so it still tests "several genuinely distinct edits, one undo per edit," which remains true. Seven new tests cover: adjacent inserts coalescing into one undo; a cursor move breaking the run; precise cursor restoration on a mid-document coalesced undo; precise cursor restoration on the matching redo; a selection-replace not coalescing with surrounding plain inserts; a backspace correctly ending an insertion run rather than joining it; and a 510-character single run correctly falling back to the clamp once part of it ages out of `Document`'s default 500-entry ring (confirmed not to panic and not to over-undo). All 7 passed on the first run -- unlike several earlier passes in this series, no real implementation bug or test-writing mistake was found here, stated plainly rather than manufacturing one. Full `cargo test --workspace --release` (75 tests in this crate's own integration suite, 0 failures workspace-wide), `clippy --all-targets`, `cargo fmt --check` clean. Live, through the actual binary: typing `"hello world"` (11 chars) as one continuous run and pressing Ctrl+Z once removed the entire string in a single step (screenshotted before/after); Ctrl+Y once restored it in full, cursor correctly back at the end (screenshotted); typing `"!"` after an intervening Left-then-Right arrow press correctly started a *new* run -- a single subsequent Ctrl+Z removed only the `"!"`, leaving `"hello world"` untouched (screenshotted), confirming the run-breaking rule holds against real, live keyboard input, not just headless test calls. The 50k-line `--synthetic:` benchmark was re-run afterward (this benchmark never calls undo/redo, so coalescing itself isn't exercised by it, only `insert_at_cursor`'s marginally larger per-call bookkeeping) and shows no regression.

**What this does not confirm.** Backspace-run coalescing (named above as a deliberate cut, tracked as a distinct, not-yet-scheduled gap, not folded into this task). No idle-timeout-based run termination -- a run only ends on a *different kind* of action, never merely on elapsed wall-clock time between keystrokes (some editors coalesce differently past a pause); not attempted here. No interaction with LSP-driven or programmatic multi-character insertions (e.g. a hypothetical future autocomplete accepting a suggestion) was tested -- `insert_at_cursor` treats any non-selection insert as run-eligible regardless of who called it or how many characters it contains, which is probably the right behavior but hasn't been exercised by anything other than single real keystrokes and the dedicated multi-char test above.

### 75.26 Real File Tree Sidebar — Task #24

Closes task #24, split off from task #16 back in §75.21 as genuinely different scope (filesystem traversal and a new open-file interaction, not visual tab-bar work). Before this pass, files could only be opened via `--open:<path>` CLI arguments at startup -- no in-app way to browse a project and open a file existed anywhere.

**A new, pure, no-GPU module, same split as `tab_bar.rs`.** `file_tree.rs` owns `FileTree` (a root path plus a `BTreeSet` of expanded directory paths) and two pure functions: `visible_rows()` (real, recursive `std::fs::read_dir` through expanded directories only, re-reading from disk every call -- no caching, a real, named v1 simplification) and `build_tree_text()` (turns a row list into the sidebar's real display string, one line per row, `"> "`/`"v "`/`"  "` ASCII markers rather than Unicode disclosure triangles so this never depends on font glyph coverage). A directory that can't be read (permissions, deleted mid-session) is silently skipped, matching this crate's established "a real I/O failure degrades gracefully" pattern rather than becoming a new exception to it.

**Hit-testing turned out simpler than the tab bar's own, not harder.** The tab bar is genuinely one line with many tabs packed into it, needing a real char-range list (`TabHit`) to resolve a click. The sidebar is genuinely multi-line -- one real text line per row -- so the same real cosmic-text `Buffer::hit` technique already used everywhere else in this crate returns a `Cursor::line` that *is* the row index directly, no range list needed at all. `TextState::hit_test_sidebar` is consequently the simplest of the three hit-test methods in this file.

**The layout shift reused an already-proven trick instead of touching every call site by hand.** §75.21 already established that redefining `TEXT_ORIGIN_Y` in terms of a new `TAB_BAR_HEIGHT` constant automatically shifts every downstream call site that already used the symbolic constant. The exact same move works horizontally: `TEXT_ORIGIN_X` is now `SIDEBAR_WIDTH + 8.0` instead of a bare `8.0`, and because the main editor's rendering, cursor position, and hit-testing *and* the tab bar's own rendering and hit-testing already routed through `TEXT_ORIGIN_X` symbolically, all of them shifted right to make room for the sidebar with zero individual changes beyond the one constant's definition.

**Real project-root reuse, not a second root concept.** `sidebar_root()` calls the exact same `language::find_project_root` (plus the same single-file-mode parent-directory fallback) `open_file()` already uses to decide where to spawn an LSP session -- the sidebar shows the same root the language server is actually analyzing, not an independently-guessed one. `None` for a `--synthetic:<n>` fixture (no real path to root a tree at), in which case the sidebar simply has nothing to show -- the same "empty is the ordinary, ungated state" pattern the tab bar and modal buffers already established, not a special case.

**Clicking an already-open file switches to it instead of duplicating it.** `find_open_file_index()` canonicalizes both the clicked path and each open tab's own label before comparing, so a relatively-launched CLI path and an absolutely-derived sidebar path for the same real file still match; it falls back to a raw path comparison if canonicalization fails on either side (e.g. a file removed out from under an already-open tab) rather than always treating that as "no match, reopen." Opening a genuinely new file from the sidebar copies the *current* `visible_lines` from the active file's `Viewport` into the new one's, since a mid-runtime file (unlike a startup `--open:` one) is created after the window size is already known, not before.

**Real, executed verification.** 8 new headless tests in `file_tree.rs` (default listing with dir-then-file sort order, collapsed children hidden, expand reveals depth+1, re-toggling collapses again, nested two-level expansion, an unreadable root returning an empty list rather than panicking, and `build_tree_text`'s exact marker/indentation output), all passing on the first run. Full `cargo test --workspace --release` (95 tests across this crate's own lib+integration suites, 0 failures workspace-wide), `clippy --all-targets`, `cargo fmt --check` clean. Live, through the actual binary against a real three-level fixture project (`Cargo.toml` + `src/{main.rs,lib.rs,sub/nested.rs}`): the sidebar rendered the real, correctly-sorted root listing on open (screenshotted); clicking `src` expanded it to reveal `sub`, `lib.rs`, `main.rs` at depth 1 (screenshotted); clicking `lib.rs` opened it as a real new tab with its real content (screenshotted); clicking `main.rs` again switched back to the already-open tab instead of creating a duplicate (screenshotted, tab count unchanged); clicking `sub` revealed `nested.rs` at depth 2 (screenshotted); a follow-up click-and-End-keypress in the main editor confirmed cursor placement and keyboard input both still work correctly through the shifted `TEXT_ORIGIN_X` layout. The 50k-line `--synthetic:` benchmark (which has no real path, so `file_tree` is `None` and the sidebar renders nothing) was re-run afterward and shows no regression (cold-open ~105ms, edit/cursor p99 in the normal 3.3-5.1ms run-to-run range, scroll p99 ~5.5ms).

**What this does not confirm.** No caching or filesystem watching -- `visible_rows()` re-reads from disk on every `RedrawRequested`, a real, deliberate v1 cost named rather than hidden (not measured separately from the rest of the frame; likely negligible for a typical project tree but untested against a very large directory). No keyboard navigation of the tree (arrow keys, Enter-to-open) -- mouse-only, matching how this crate's other pointer-driven UI (tab clicks, selection drags) also shipped mouse-first. No delete/rename/create/context-menu operations. No git-status decoration, no file-type icons. No horizontal scrolling or truncation if a deeply nested or long-named entry would overflow `SIDEBAR_WIDTH` -- untested at that scale. No sidebar toggle/hide -- it's always shown once a root is known, taking a fixed 200px regardless of window width, including on a narrow window where that could meaningfully crowd the editor (untested at that scale either).

---

*End of spec.*
