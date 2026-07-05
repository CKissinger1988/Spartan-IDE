import React, { useState, useEffect, useRef } from "react";
import {
  Sparkles,
  Code2,
  PenTool,
  Settings2,
  Send,
  Check,
  X,
  Circle,
  CheckCircle2,
  Folder,
  FolderOpen,
  FileCode,
  Terminal,
  Cloud,
  HardDrive,
  Layers,
  Palette,
  Plus,
  Clock,
  ShieldCheck,
  Search,
  ChevronRight,
  GitBranch,
  AlertTriangle,
  Component as ComponentIcon,
  Wand2,
  LayoutPanelLeft,
  Download,
  Trash2,
  Smartphone,
  RotateCw,
  Camera,
  Video,
  Eye,
  EyeOff,
  KeyRound,
  TerminalSquare,
  ShieldAlert,
  Slash,
  Monitor,
  Boxes,
  Cable,
  MonitorPlay,
  Cpu,
  RefreshCw,
  History,
  MessageSquare,
  Bug,
  Radio,
  Binary,
} from "lucide-react";

/* ---------- design tokens ---------- */
// §50.3: bg/surface/border tightened to match Antigravity 2.0's researched
// high-contrast token values (#09090B / #18181B / #27272A) almost exactly —
// the accent hue stays Spartan's own (§50.3 names this one deliberate
// divergence rather than leaving it an unexplained mismatch). textDim's
// luminance is raised from the previous pass for the same "high contrast,
// not just dark" goal, since it's the color most often used for body text.
const C = {
  bg: "#09090B",
  s1: "#141416",
  s2: "#18181B",
  s3: "#202024",
  border: "#27272A",
  borderLt: "#35353A",
  text: "#E9E7E4",
  textMid: "#A6A5A2",
  textDim: "#84838A",
  accent: "#C4432B",
  accentDim: "#8F3323",
  accentBg: "rgba(196,67,43,0.13)",
  accentBorder: "rgba(196,67,43,0.4)",
  green: "#4E9E72",
  greenBg: "rgba(78,158,114,0.13)",
  amber: "#C99A3D",
  amberBg: "rgba(201,154,61,0.13)",
  red: "#B2453B",
  redBg: "rgba(178,69,59,0.13)",
  teal: "#3D9C93",
  tealBg: "rgba(61,156,147,0.13)",
};
const F_UI = "'Space Grotesk', ui-sans-serif, system-ui, sans-serif";
const F_MONO = "'IBM Plex Mono', ui-monospace, monospace";

/* ---------- static content ---------- */
const SESSIONS = [
  { id: "s1", title: "Refactor auth to JWT", time: "2m ago", status: "running" },
  { id: "s2", title: "Fix flaky checkout test", time: "18m ago", status: "review" },
  { id: "s3", title: "Add Compose settings screen", time: "1h ago", status: "done" },
  { id: "s4", title: "Investigate memory leak", time: "Yesterday", status: "paused" },
  // Fleet sessions (§52) share this same list — a supervised third-party CLI
  // process, not a Leo session, tagged with its engine instead of a model badge.
  { id: "s5", title: "Bulk-rename API routes", time: "5m ago", status: "running", kind: "fleet", engine: "codex" },
  { id: "s6", title: "Migrate lockfile to pnpm", time: "3h ago", status: "done", kind: "fleet", engine: "aider" },
  {
    id: "s7",
    title: "Draft release notes for v0.9",
    time: "22m ago",
    status: "review",
    kind: "fleet",
    engine: "gemini-cli",
  },
];

const STATUS_META = {
  running: { color: C.accent, label: "Running" },
  review: { color: C.amber, label: "Needs review" },
  done: { color: C.green, label: "Done" },
  paused: { color: C.textDim, label: "Paused" },
};

// §52.2's FleetEngineProfile.color_tag, applied per-engine in the session
// rail and the Fleet session header — visually distinct from Leo's accent
// so "whose work is this" is legible at a glance across the whole rail.
// Full roster matches the legacy console's fleet (§55's parity matrix) —
// every engine that product could launch gets a registry entry here too,
// not just the two or three that happen to be running in the demo content.
const ENGINE_META = {
  codex: { color: C.teal, bg: C.tealBg, border: "rgba(61,156,147,0.35)", label: "Codex" },
  aider: { color: C.amber, bg: C.amberBg, border: "rgba(201,154,61,0.35)", label: "Aider" },
  "gemini-cli": { color: C.green, bg: C.greenBg, border: "rgba(78,158,114,0.35)", label: "Gemini CLI" },
  opencode: { color: C.teal, bg: C.tealBg, border: "rgba(61,156,147,0.35)", label: "OpenCode" },
  "copilot-cli": { color: C.textMid, bg: C.s3, border: C.borderLt, label: "Copilot CLI" },
  "cursor-agent": { color: C.accent, bg: C.accentBg, border: C.accentBorder, label: "Cursor Agent" },
  "qwen-code": { color: C.amber, bg: C.amberBg, border: "rgba(201,154,61,0.35)", label: "Qwen Code" },
  amp: { color: C.green, bg: C.greenBg, border: "rgba(78,158,114,0.35)", label: "Amp" },
  goose: { color: C.teal, bg: C.tealBg, border: "rgba(61,156,147,0.35)", label: "Goose" },
  crush: { color: C.accent, bg: C.accentBg, border: C.accentBorder, label: "Crush" },
  continue: { color: C.textMid, bg: C.s3, border: C.borderLt, label: "Continue" },
  openhands: { color: C.green, bg: C.greenBg, border: "rgba(78,158,114,0.35)", label: "OpenHands" },
  "openai-cli": { color: C.amber, bg: C.amberBg, border: "rgba(201,154,61,0.35)", label: "OpenAI CLI" },
  "claude-code": { color: C.accent, bg: C.accentBg, border: C.accentBorder, label: "Claude Code" },
  cline: { color: C.teal, bg: C.tealBg, border: "rgba(61,156,147,0.35)", label: "Cline" },
  windsurf: { color: C.green, bg: C.greenBg, border: "rgba(78,158,114,0.35)", label: "Windsurf" },
};

const PLAN_STEPS = ["Understand", "Plan", "Implement", "Verify", "Done"];

const SESSION_CONTENT = {
  s1: {
    planTitle: "Refactor session auth to JWT",
    scope: "4 files · 2 risk notes",
    tasks: [
      { label: "Replace session middleware with JWT verify", done: true },
      { label: "Update login/refresh route handlers", done: true },
      { label: "Migrate existing session store reads", done: false },
      { label: "Add expiry + rotation tests", done: false },
    ],
    diffs: [
      { file: "src/auth/middleware.rs", add: 42, del: 18 },
      { file: "src/auth/routes.rs", add: 27, del: 9 },
      { file: "src/auth/session_store.rs", add: 6, del: 31 },
    ],
    verification: { pass: 18, fail: 1, running: false },
    // §71: Antigravity's third core artifact alongside plan/task is a
    // walkthrough — a written recap after the fact, not just a live plan.
    walkthrough:
      "Replaced the session-cookie auth path with JWT verification in middleware.rs, keeping the existing refresh-token flow so no client changes are needed. routes.rs now issues/refreshes JWTs instead of session IDs. session_store.rs migration is the one piece still in flight — the remaining read call sites need to switch from session lookups to JWT claims before this is fully done.",
    chat: [
      { from: "user", text: "Switch our session auth over to JWT, keep the refresh flow." },
      {
        from: "leo",
        text: "Found 3 call sites touching the session store directly. Plan's in the Auxiliary Pane — want me to start once you approve it?",
      },
      { from: "user", text: "Approved, go ahead." },
      { from: "leo", text: "On it. Middleware and routes are done, migrating the remaining session reads now." },
    ],
  },
  s2: {
    planTitle: "Root-cause flaky checkout test",
    scope: "1 file · timing-related",
    tasks: [
      { label: "Reproduce failure locally (12 runs)", done: true },
      { label: "Bisect against last 8 commits", done: true },
      { label: "Confirm race in async cart total", done: true },
      { label: "Apply fix + re-run 50x", done: false },
    ],
    diffs: [{ file: "tests/checkout.spec.ts", add: 11, del: 3 }],
    verification: { pass: 47, fail: 3, running: true },
    walkthrough:
      "Bisected 8 commits back and reproduced the failure locally 12/12 times before touching anything. Root cause: the cart total recompute and the discount effect both write to the same derived total without a defined order, so the test occasionally reads a half-updated value. Fix serializes the two updates instead of racing them; re-running 50x locally now before calling this done.",
    chat: [
      { from: "user", text: "This checkout test keeps failing intermittently in CI." },
      {
        from: "leo",
        text: "Bisected it — race between the cart total recompute and the discount effect. Historically flaky, not a new regression, flagging that in the report.",
      },
      { from: "leo", text: "Applying a fix now, re-running 50x locally before I call it done." },
    ],
  },
};
SESSION_CONTENT.s3 = {
  ...SESSION_CONTENT.s1,
  planTitle: "Add Compose settings screen",
  scope: "3 files · shipped",
  diffs: SESSION_CONTENT.s1.diffs.map((d) => ({ ...d, accepted: true })),
  chat: [
    { from: "user", text: "Add a basic settings screen, Compose, matching the existing style." },
    { from: "leo", text: "Done — screen's live in the emulator, screenshot verified against the design tokens." },
  ],
};
SESSION_CONTENT.s4 = {
  ...SESSION_CONTENT.s2,
  planTitle: "Investigate memory leak",
  scope: "paused · awaiting heap snapshot",
  chat: [
    { from: "user", text: "Memory climbs steadily after ~20 min of use, can you dig in?" },
    {
      from: "leo",
      text: "Narrowed it to the event listener cleanup path, paused here — I need a heap snapshot from a longer session to confirm before proposing a fix.",
    },
  ],
};

// Fleet sessions carry a different shape entirely (§52.1-52.2) — a
// supervised PTY transcript and self-reported usage, not a plan/diff
// artifact set, since Spartan doesn't control a third-party CLI's internal
// tool-call loop the way it does Leo's.
SESSION_CONTENT.s5 = {
  scope: "codex · /workspace/spartan-web · attached",
  terminal: [
    { text: '$ codex "rename every /api/v1/* route to /api/v2/* and update callers"', tone: "cmd" },
    { text: "› scanning 34 route definitions across 6 files…", tone: "dim" },
    { text: "› src/routes/users.ts: 4 routes renamed", tone: "normal" },
    { text: "› src/routes/orders.ts: 7 routes renamed", tone: "normal" },
    { text: "› updating 11 caller sites in src/client/*…", tone: "normal" },
    { text: "› awaiting approval to write 6 files (destructive: no)", tone: "amber" },
  ],
  usage: { tokensSelfReported: "≈48k", requests: 12, quotaState: "ok" },
};
SESSION_CONTENT.s6 = {
  scope: "aider · /workspace/spartan-web · detached",
  terminal: [
    { text: '$ aider --message "migrate the lockfile from npm to pnpm, keep versions pinned"', tone: "cmd" },
    { text: "› converted package-lock.json → pnpm-lock.yaml", tone: "normal" },
    { text: "› updated CI workflow to actions/setup-node with pnpm cache", tone: "normal" },
    { text: "› done — 3 files changed", tone: "green" },
  ],
  usage: { tokensSelfReported: "≈21k", requests: 5, quotaState: "ok" },
};
SESSION_CONTENT.s7 = {
  scope: "gemini-cli · /workspace/spartan-web · attached",
  terminal: [
    { text: '$ gemini "draft release notes for v0.9 from the last 40 commits"', tone: "cmd" },
    { text: "› reading git log v0.8..HEAD (40 commits)…", tone: "dim" },
    { text: "› grouped into Features (6), Fixes (11), Breaking (1)", tone: "normal" },
    { text: "› draft written to RELEASE_NOTES.md — awaiting your review", tone: "amber" },
  ],
  usage: { tokensSelfReported: "≈9k", requests: 3, quotaState: "ok" },
};

/* ---------- small pieces ---------- */
function ModelBadge({ provider }) {
  const local = provider === "local";
  return (
    <span
      className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs"
      style={{
        background: local ? C.tealBg : C.accentBg,
        color: local ? C.teal : C.accent,
        border: `1px solid ${local ? "rgba(61,156,147,0.35)" : C.accentBorder}`,
        fontFamily: F_MONO,
      }}
    >
      {local ? <HardDrive size={11} /> : <Cloud size={11} />}
      Leo · {local ? "Qwen2.5-Coder (local)" : "Claude Sonnet"}
    </span>
  );
}

function FleetBadge({ engine }) {
  const meta = ENGINE_META[engine] || { color: C.textDim, bg: C.s3, border: C.borderLt, label: engine };
  return (
    <span
      className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs"
      style={{
        background: meta.bg,
        color: meta.color,
        border: `1px solid ${meta.border}`,
        fontFamily: F_MONO,
      }}
    >
      <Terminal size={11} />
      Fleet · {meta.label}
    </span>
  );
}

function StatusDot({ status }) {
  const meta = STATUS_META[status];
  return (
    <span
      className="inline-block rounded-full flex-shrink-0"
      style={{
        width: 7,
        height: 7,
        background: meta.color,
        boxShadow: status === "running" ? `0 0 0 3px ${C.accentBg}` : "none",
      }}
    />
  );
}

/* ---------- top rail ---------- */
const MORE_VIEWS = [
  { id: "test", label: "Test", icon: Layers },
  { id: "ops", label: "Ops", icon: GitBranch },
  { id: "data", label: "Data", icon: HardDrive },
  { id: "manage", label: "Manage", icon: Clock },
];

// §62.2: every entry here is hideable two ways — from this central View
// menu, and from a hide button on the panel itself. The View menu is also
// the only way back once a panel is hidden, so it must list everything.
const PANEL_LABELS = {
  leftRail: "Left Rail",
  auxPane: "Auxiliary Pane",
  fileTree: "File Tree",
  planTracker: "Plan Tracker",
};

function TopRail({
  mode,
  setMode,
  provider,
  setProvider,
  onPalette,
  onSettings,
  panelVisibility,
  onTogglePanel,
  devMode,
}) {
  const [moreOpen, setMoreOpen] = useState(false);
  const [viewOpen, setViewOpen] = useState(false);
  const modes = [
    { id: "agent", label: "Agent", icon: Sparkles },
    { id: "editor", label: "Editor", icon: Code2 },
    { id: "design", label: "Design", icon: PenTool },
  ];
  const inMore = MORE_VIEWS.some((v) => v.id === mode);
  return (
    <div
      className="flex items-center justify-between px-4 flex-shrink-0"
      style={{ height: 52, borderBottom: `1px solid ${C.border}`, background: C.s1 }}
    >
      <div className="flex items-center gap-3">
        <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
          <path
            d="M12 2L21 6.5V13C21 18 17 21.5 12 23C7 21.5 3 18 3 13V6.5L12 2Z"
            stroke={C.accent}
            strokeWidth="1.6"
          />
          <circle cx="12" cy="12" r="2.4" fill={C.accent} />
        </svg>
        <div className="flex items-baseline gap-1.5">
          <span style={{ fontFamily: F_UI, fontWeight: 600, color: C.text, fontSize: 15, letterSpacing: "-0.01em" }}>
            Spartan
          </span>
          <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim, letterSpacing: "0.05em" }}>IDE</span>
        </div>
      </div>

      <div className="flex items-center gap-1 relative">
        <div
          className="flex items-center gap-0.5 rounded-lg p-0.5"
          style={{ background: C.s2, border: `1px solid ${C.border}` }}
        >
          {modes.map((m) => {
            const active = mode === m.id;
            const Icon = m.icon;
            return (
              <button
                key={m.id}
                onClick={() => {
                  setMode(m.id);
                  setMoreOpen(false);
                }}
                className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs transition-all duration-150"
                style={{
                  fontFamily: F_UI,
                  fontWeight: 500,
                  background: active ? C.accentBg : "transparent",
                  color: active ? C.accent : C.textMid,
                  border: `1px solid ${active ? C.accentBorder : "transparent"}`,
                }}
              >
                <Icon size={13} />
                {m.label} View
              </button>
            );
          })}
          <button
            onClick={() => setMoreOpen((o) => !o)}
            className="flex items-center gap-1 rounded-md px-2.5 py-1.5 text-xs"
            style={{
              fontFamily: F_UI,
              fontWeight: 500,
              background: inMore ? C.accentBg : "transparent",
              color: inMore ? C.accent : C.textDim,
              border: `1px solid ${inMore ? C.accentBorder : "transparent"}`,
            }}
          >
            {inMore ? MORE_VIEWS.find((v) => v.id === mode).label : "More"}{" "}
            <ChevronRight size={11} style={{ transform: "rotate(90deg)" }} />
          </button>
        </div>

        {moreOpen && (
          <div
            className="absolute top-full mt-1 right-0 rounded-lg overflow-hidden fade-in-up z-40"
            style={{ background: C.s2, border: `1px solid ${C.borderLt}`, minWidth: 160 }}
          >
            {MORE_VIEWS.map((v) => {
              const Icon = v.icon;
              return (
                <button
                  key={v.id}
                  onClick={() => {
                    setMode(v.id);
                    setMoreOpen(false);
                  }}
                  className="w-full flex items-center gap-2 px-3 py-2 text-left"
                  style={{
                    background: mode === v.id ? C.s3 : "transparent",
                    color: C.text,
                    fontFamily: F_UI,
                    fontSize: 12,
                  }}
                >
                  <Icon size={12} color={C.textDim} /> {v.label} View
                </button>
              );
            })}
            <div className="px-3 py-2" style={{ borderTop: `1px solid ${C.border}` }}>
              <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>+ add view (§22)</span>
            </div>
          </div>
        )}
      </div>

      <div className="flex items-center gap-2">
        {devMode && (
          <span
            className="flex items-center gap-1 rounded-md px-2 py-1 fade-in-up"
            style={{
              background: C.redBg,
              border: "1px solid rgba(178,69,59,0.4)",
              color: C.red,
              fontFamily: F_MONO,
              fontSize: 10,
            }}
            title="Developer Mode is active for this workspace — §60. No path jail, but destructive-action approval and plugin sandboxing are unaffected (§60.1)."
          >
            <ShieldAlert size={12} /> DEV MODE
          </span>
        )}
        <button
          onClick={onPalette}
          className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs"
          style={{ background: C.s2, border: `1px solid ${C.border}`, color: C.textDim, fontFamily: F_MONO }}
        >
          <Search size={12} />
          <span>Search or ask Leo</span>
          <span className="rounded px-1" style={{ background: C.s3, fontSize: 10 }}>
            ⌘K
          </span>
        </button>
        <button
          onClick={() => setProvider(provider === "cloud" ? "local" : "cloud")}
          className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs"
          style={{ background: C.s2, border: `1px solid ${C.border}`, color: C.textMid, fontFamily: F_MONO }}
          title="Toggle Leo's active backend"
        >
          {provider === "cloud" ? <Cloud size={13} /> : <HardDrive size={13} />}
          {provider === "cloud" ? "Cloud" : "Local"}
        </button>
        <div className="relative">
          <button
            onClick={() => setViewOpen((o) => !o)}
            className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs"
            style={{
              background: viewOpen ? C.s3 : C.s2,
              border: `1px solid ${C.border}`,
              color: C.textMid,
              fontFamily: F_MONO,
            }}
            title="Show/hide panels (§62.2)"
          >
            <Eye size={13} /> View
          </button>
          {viewOpen && (
            <div
              className="absolute top-full mt-1 right-0 rounded-lg overflow-hidden fade-in-up z-40"
              style={{ background: C.s2, border: `1px solid ${C.borderLt}`, minWidth: 170 }}
            >
              {Object.entries(PANEL_LABELS).map(([key, label]) => {
                const visible = panelVisibility[key];
                return (
                  <button
                    key={key}
                    onClick={() => onTogglePanel(key)}
                    className="w-full flex items-center gap-2 px-3 py-2 text-left"
                    style={{ color: C.text, fontFamily: F_UI, fontSize: 12 }}
                  >
                    {visible ? <Eye size={12} color={C.accent} /> : <EyeOff size={12} color={C.textDim} />}
                    <span style={{ color: visible ? C.text : C.textDim }}>{label}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
        <button onClick={onSettings} title="Settings">
          <Settings2 size={16} color={C.textDim} className="cursor-pointer" />
        </button>
        <div
          className="rounded-full flex items-center justify-center"
          style={{
            width: 26,
            height: 26,
            background: C.s3,
            border: `1px solid ${C.border}`,
            fontSize: 11,
            color: C.textMid,
            fontFamily: F_UI,
          }}
        >
          JD
        </div>
      </div>
    </div>
  );
}

/* ---------- left rail ---------- */
const WORKSPACES = [
  { id: "w1", name: "spartan-web", active: true },
  { id: "w2", name: "spartan-mobile", active: false },
  // §61.1: a WSL distro is offered as a workspace root the same way an
  // SSH-remote or container target already is, through the VFS abstraction
  // (§37.1) — not a separate mode with its own UI.
  { id: "w3", name: "spartan-api (WSL: Ubuntu-22.04)", active: false, kind: "wsl" },
];

function LeftRail({ collapsed, setCollapsed, activeId, setActiveId, onHide }) {
  return (
    <div
      className="flex flex-col flex-shrink-0 transition-all duration-200"
      style={{ width: collapsed ? 56 : 240, borderRight: `1px solid ${C.border}`, background: C.s1 }}
    >
      {/* Workspaces + Playground -- mirrors Antigravity 2.0's actual left column */}
      {!collapsed && (
        <div className="px-3 pt-3 pb-2" style={{ borderBottom: `1px solid ${C.border}` }}>
          <span
            style={{ fontFamily: F_UI, fontSize: 10.5, fontWeight: 600, color: C.textDim, letterSpacing: "0.06em" }}
          >
            WORKSPACES
          </span>
          <div className="mt-1.5 space-y-0.5">
            {WORKSPACES.map((w) => (
              <div
                key={w.id}
                className="flex items-center gap-1.5 rounded-md px-1.5 py-1 cursor-pointer"
                style={{ background: w.active ? C.s3 : "transparent" }}
                title={w.kind === "wsl" ? "WSL-backed workspace root (§61.1)" : undefined}
              >
                {w.kind === "wsl" ? (
                  <Monitor size={11} color={w.active ? C.teal : C.textDim} />
                ) : (
                  <Folder size={11} color={w.active ? C.accent : C.textDim} />
                )}
                <span
                  className="truncate"
                  style={{ fontFamily: F_MONO, fontSize: 11, color: w.active ? C.text : C.textMid }}
                >
                  {w.name}
                </span>
              </div>
            ))}
            <div
              className="flex items-center gap-1.5 rounded-md px-1.5 py-1 cursor-pointer"
              style={{ border: `1px dashed ${C.borderLt}` }}
            >
              <PenTool size={10} color={C.textDim} />
              <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>Playground · no consequences</span>
            </div>
          </div>
        </div>
      )}

      <div className="flex items-center justify-between px-3 py-2.5">
        {!collapsed && (
          <span style={{ fontFamily: F_UI, fontSize: 11, fontWeight: 600, color: C.textDim, letterSpacing: "0.06em" }}>
            INBOX
          </span>
        )}
        <div className="flex items-center gap-1.5">
          {!collapsed && onHide && (
            <button onClick={onHide} title="Hide panel — bring back via View (§62.2)" style={{ color: C.textDim }}>
              <EyeOff size={13} />
            </button>
          )}
          <button onClick={() => setCollapsed(!collapsed)} style={{ color: C.textDim }}>
            <LayoutPanelLeft size={14} />
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-2 space-y-0.5">
        {SESSIONS.map((s) => {
          const active = activeId === s.id;
          return (
            <button
              key={s.id}
              onClick={() => setActiveId(s.id)}
              className="w-full flex items-center gap-2 rounded-md px-2 py-2 text-left transition-colors duration-150"
              style={{
                background: active ? C.s3 : "transparent",
                borderLeft: `2px solid ${active ? C.accent : "transparent"}`,
              }}
            >
              <StatusDot status={s.status} />
              {!collapsed && (
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <div
                      className="truncate"
                      style={{ fontFamily: F_UI, fontSize: 12.5, color: active ? C.text : C.textMid }}
                    >
                      {s.title}
                    </div>
                    {s.kind === "fleet" && (
                      <span
                        className="flex-shrink-0 rounded px-1"
                        style={{
                          fontFamily: F_MONO,
                          fontSize: 9,
                          color: (ENGINE_META[s.engine] || {}).color || C.textDim,
                          background: (ENGINE_META[s.engine] || {}).bg || C.s3,
                        }}
                      >
                        {(ENGINE_META[s.engine] || {}).label || s.engine}
                      </span>
                    )}
                  </div>
                  <div style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>{s.time}</div>
                </div>
              )}
            </button>
          );
        })}
      </div>

      {!collapsed && (
        <div className="px-3 py-3" style={{ borderTop: `1px solid ${C.border}` }}>
          <div className="flex items-center gap-1.5 mb-1.5" style={{ color: C.textDim }}>
            <Clock size={11} />
            <span style={{ fontFamily: F_UI, fontSize: 10.5, fontWeight: 600, letterSpacing: "0.05em" }}>
              SCHEDULED
            </span>
          </div>
          <div style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>Dependency audit · 9:00 AM daily</div>
        </div>
      )}
    </div>
  );
}

/* ---------- plan step tracker ---------- */
function PlanTracker({ step, onHide }) {
  return (
    <div className="flex items-center gap-1 px-5 py-3 group" style={{ borderBottom: `1px solid ${C.border}` }}>
      {PLAN_STEPS.map((label, i) => {
        const done = i < step;
        const current = i === step;
        return (
          <React.Fragment key={label}>
            <div className="flex items-center gap-1.5">
              <div
                className={current ? "pulse-glow" : ""}
                style={{
                  width: 7,
                  height: 7,
                  borderRadius: 99,
                  background: done || current ? C.accent : C.s3,
                  border: `1px solid ${done || current ? C.accent : C.borderLt}`,
                }}
              />
              <span
                style={{ fontFamily: F_MONO, fontSize: 11, color: done ? C.textMid : current ? C.text : C.textDim }}
              >
                {label}
              </span>
            </div>
            {i < PLAN_STEPS.length - 1 && (
              <div className="flex-1" style={{ height: 1, background: done ? C.accentDim : C.border, minWidth: 16 }} />
            )}
          </React.Fragment>
        );
      })}
      {onHide && (
        <button
          onClick={onHide}
          title="Hide panel — bring back via View (§62.2)"
          className="ml-3 flex-shrink-0"
          style={{ color: C.textDim }}
        >
          <EyeOff size={12} />
        </button>
      )}
    </div>
  );
}

/* ---------- agent view ---------- */
const TERMINAL_TONE_COLOR = {
  cmd: C.text,
  normal: C.textMid,
  dim: C.textDim,
  amber: C.amber,
  green: C.green,
};

// §52.1: a Fleet session is a supervised PTY transcript, not a Leo plan/chat
// loop — no plan tracker, no ModelBadge, no structured chat turns, since
// Spartan doesn't converse with a third-party CLI token-by-token.
function FleetSessionView({ session }) {
  const content = SESSION_CONTENT[session.id];
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-6 py-3" style={{ borderBottom: `1px solid ${C.border}` }}>
        <FleetBadge engine={session.engine} />
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>{content.scope}</span>
      </div>
      <div
        className="flex-1 overflow-y-auto px-6 py-5"
        style={{ background: "#08080A", fontFamily: F_MONO, fontSize: 12.5, lineHeight: 1.7 }}
      >
        {content.terminal.map((line, i) => (
          <div key={i} className="fade-in-up" style={{ color: TERMINAL_TONE_COLOR[line.tone] || C.textMid }}>
            {line.text}
          </div>
        ))}
      </div>
      <div className="px-6 py-4" style={{ borderTop: `1px solid ${C.border}` }}>
        <div
          className="flex items-center gap-2 rounded-xl px-3.5 py-2.5"
          style={{ background: C.s2, border: `1px solid ${C.border}` }}
        >
          <Terminal size={14} color={C.textDim} />
          <input
            placeholder={`Send input to the ${(ENGINE_META[session.engine] || {}).label || session.engine} session…`}
            className="flex-1 bg-transparent outline-none"
            style={{ fontFamily: F_MONO, fontSize: 12.5, color: C.text }}
          />
          <Send size={15} color={C.textDim} className="cursor-pointer" />
        </div>
      </div>
    </div>
  );
}

// §62.1 — inline, scoped to this input, distinct from the modal ⌘K palette
// (§16.1). Every command is a thin dispatcher onto something that already
// exists elsewhere in the spec, never new plumbing of its own.
const SLASH_COMMANDS = [
  { cmd: "/plan", desc: "Produce an ImplementationPlan artifact before touching any file" },
  { cmd: "/test", desc: "Run the project's configured test task (§20.2)" },
  { cmd: "/commit", desc: "Open Source Control's commit view, pre-focused (§56.1)" },
  { cmd: "/pr", desc: "Open the Pull Request tab, starting create-PR if none exists (§56.3)" },
  { cmd: "/explain", desc: "Explain the current selection, or what's on screen with none" },
  { cmd: "/model", desc: "Jump to the Leo & Models picker for a fast mid-session switch" },
  { cmd: "/vibe", desc: "Run this task in Vibe Mode (§45), just for this task" },
  { cmd: "/undo", desc: "Roll back to the last checkpoint (§4.2)" },
  { cmd: "/clear", desc: "Clear this visible transcript (display-only, §7's log is untouched)" },
];

function AgentView({ session, planStep, provider, panelVisibility = {}, onTogglePanel }) {
  if (session.kind === "fleet") {
    return <FleetSessionView session={session} />;
  }
  const content = SESSION_CONTENT[session.id];
  const [chatInput, setChatInput] = useState("");
  const slashOpen = chatInput.startsWith("/");
  const filteredCommands = slashOpen ? SLASH_COMMANDS.filter((c) => c.cmd.startsWith(chatInput.split(" ")[0])) : [];

  return (
    <div className="flex flex-col h-full">
      {panelVisibility.planTracker !== false && (
        <PlanTracker step={planStep} onHide={onTogglePanel ? () => onTogglePanel("planTracker") : undefined} />
      )}
      <div className="flex-1 overflow-y-auto px-6 py-5 space-y-4">
        {content.chat.map((m, i) => (
          <div key={i} className={`flex ${m.from === "user" ? "justify-end" : "justify-start"} fade-in-up`}>
            <div className="max-w-lg">
              {m.from === "leo" && (
                <div className="mb-1.5">
                  <ModelBadge provider={provider} />
                </div>
              )}
              <div
                className="rounded-xl px-3.5 py-2.5"
                style={{
                  background: m.from === "user" ? C.s3 : C.s2,
                  border: `1px solid ${m.from === "user" ? C.borderLt : C.border}`,
                  fontFamily: F_UI,
                  fontSize: 13,
                  color: C.text,
                  lineHeight: 1.5,
                }}
              >
                {m.text}
              </div>
            </div>
          </div>
        ))}
      </div>
      <div className="px-6 py-4 relative" style={{ borderTop: `1px solid ${C.border}` }}>
        {slashOpen && filteredCommands.length > 0 && (
          <div
            className="absolute left-6 right-6 rounded-lg overflow-hidden fade-in-up z-20"
            style={{ bottom: "100%", marginBottom: 8, background: C.s2, border: `1px solid ${C.borderLt}` }}
          >
            {filteredCommands.map((c) => (
              <button
                key={c.cmd}
                onClick={() => setChatInput(c.cmd + " ")}
                className="w-full flex items-center gap-3 px-3 py-2 text-left"
                style={{ background: "transparent", borderBottom: `1px solid ${C.border}` }}
              >
                <Slash size={11} color={C.accent} className="flex-shrink-0" />
                <span style={{ fontFamily: F_MONO, fontSize: 12, color: C.text, flexShrink: 0, width: 80 }}>
                  {c.cmd}
                </span>
                <span style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim }}>{c.desc}</span>
              </button>
            ))}
          </div>
        )}
        <div
          className="flex items-center gap-2 rounded-xl px-3.5 py-2.5"
          style={{ background: C.s2, border: `1px solid ${C.border}` }}
        >
          <input
            value={chatInput}
            onChange={(e) => setChatInput(e.target.value)}
            placeholder="Ask Leo or describe a task… ( / for quick commands )"
            className="flex-1 bg-transparent outline-none"
            style={{ fontFamily: F_UI, fontSize: 13, color: C.text }}
          />
          <Send size={15} color={C.accent} className="cursor-pointer" />
        </div>
      </div>
    </div>
  );
}

/* ---------- editor view ---------- */
const FILES = [
  { name: "src", type: "folder", open: true },
  { name: "auth", type: "folder", open: true, indent: 1 },
  { name: "middleware.rs", type: "file", indent: 2, active: true, tick: "accent" },
  { name: "routes.rs", type: "file", indent: 2 },
  { name: "session_store.rs", type: "file", indent: 2, tick: "diag" },
  { name: "lib.rs", type: "file", indent: 1 },
  { name: "Cargo.toml", type: "file" },
];

const CODE_LINES = [
  { n: 41, t: "pub fn verify_token(header: &HeaderValue) -> Result<Claims, AuthError> {" },
  { n: 42, t: "    let token = strip_bearer(header)?;", tick: "accent" },
  { n: 43, t: "    let claims = decode::<Claims>(&token, &DECODING_KEY, &Validation::default())?;" },
  { n: 44, t: "    if claims.exp < now() {", diag: true },
  { n: 45, t: "        return Err(AuthError::Expired);" },
  { n: 46, t: "    }" },
  { n: 47, t: "    Ok(claims.claims)" },
  { n: 48, t: "}" },
];

function DebugConsole({ hit }) {
  return (
    <div className="flex h-full">
      <div className="w-40 flex-shrink-0 overflow-y-auto py-2" style={{ borderRight: `1px solid ${C.border}` }}>
        <div
          className="px-3 pb-1"
          style={{ fontFamily: F_UI, fontSize: 10, fontWeight: 600, color: C.textDim, letterSpacing: "0.05em" }}
        >
          CALL STACK
        </div>
        {["verify_token()", "authenticate()", "handle_request()"].map((f, i) => (
          <div key={f} className="px-3 py-1" style={{ background: i === 0 ? C.s3 : "transparent" }}>
            <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: i === 0 ? C.accent : C.textMid }}>{f}</span>
          </div>
        ))}
      </div>
      <div className="flex-1 overflow-y-auto py-2 px-3">
        <div
          style={{
            fontFamily: F_UI,
            fontSize: 10,
            fontWeight: 600,
            color: C.textDim,
            letterSpacing: "0.05em",
            marginBottom: 4,
          }}
        >
          VARIABLES
        </div>
        {hit ? (
          <div className="space-y-1">
            {[
              ["token", '"eyJhbGci..."'],
              ["claims.exp", "1719900000"],
              ["now()", "1719900042"],
            ].map(([k, v]) => (
              <div key={k} className="flex items-center gap-2">
                <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.teal }}>{k}</span>
                <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.text }}>{v}</span>
              </div>
            ))}
          </div>
        ) : (
          <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>
            Not paused — run to a breakpoint (§32)
          </span>
        )}
      </div>
    </div>
  );
}

const LOGCAT_LINES = [
  { tag: "AuthService", level: "I", msg: "Token refresh succeeded for session 8f21" },
  { tag: "OkHttp", level: "D", msg: "--> POST /v1/refresh" },
  { tag: "AuthService", level: "E", msg: "Expired claim rejected, exp=1719900000" },
];
// §33.7's Screen Tab: screenshot/screenrecord one-click capture plus a live,
// interactive scrcpy-style mirror — clicks in the mirror translate to
// `input tap`, not just a periodic screenshot refresh.
function ScreenMirrorTab() {
  const [recording, setRecording] = useState(false);
  const [flash, setFlash] = useState(false);
  const [lastTap, setLastTap] = useState(null);

  function takeScreenshot() {
    setFlash(true);
    setTimeout(() => setFlash(false), 220);
  }
  function handleMirrorClick(e) {
    const rect = e.currentTarget.getBoundingClientRect();
    const x = Math.round(((e.clientX - rect.left) / rect.width) * 1080);
    const y = Math.round(((e.clientY - rect.top) / rect.height) * 2400);
    setLastTap({ x, y });
  }

  return (
    <div className="flex gap-4 p-3 flex-1 min-h-0">
      <div className="flex flex-col items-center flex-shrink-0">
        <div
          onClick={handleMirrorClick}
          className="relative rounded-2xl overflow-hidden cursor-crosshair fade-in-up"
          style={{
            width: 132,
            height: 280,
            background: "#050506",
            border: `2px solid ${C.borderLt}`,
            boxShadow: flash ? `0 0 0 3px ${C.accentBorder}` : "none",
          }}
          title="Live mirror — click translates to input tap (§33.7)"
        >
          <div className="flex items-center justify-between px-2" style={{ height: 14, background: "#000" }}>
            <span style={{ fontFamily: F_MONO, fontSize: 6.5, color: C.textMid }}>9:41</span>
            <span style={{ fontFamily: F_MONO, fontSize: 6.5, color: C.textMid }}>▮▮▮ 82%</span>
          </div>
          <div
            className="px-2 py-2"
            style={{ height: 252, background: "linear-gradient(180deg, #101012 0%, #08080a 100%)" }}
          >
            <div style={{ fontFamily: F_UI, fontWeight: 600, fontSize: 8.5, color: C.text, marginBottom: 6 }}>
              Spartan Demo
            </div>
            <div className="rounded" style={{ height: 5, width: "70%", background: C.s3, marginBottom: 4 }} />
            <div className="rounded" style={{ height: 5, width: "45%", background: C.s3, marginBottom: 10 }} />
            <div
              className="rounded-md flex items-center justify-center"
              style={{ height: 22, background: C.accentBg, border: `1px solid ${C.accentBorder}`, marginBottom: 6 }}
            >
              <span style={{ fontFamily: F_UI, fontSize: 7, color: C.accent }}>Sign in</span>
            </div>
            {lastTap && (
              <span
                className="absolute rounded-full fade-in-up"
                style={{
                  width: 10,
                  height: 10,
                  marginLeft: -5,
                  marginTop: -5,
                  left: `${(lastTap.x / 1080) * 100}%`,
                  top: `${14 + (lastTap.y / 2400) * 86}%`,
                  border: `2px solid ${C.accent}`,
                }}
              />
            )}
          </div>
          {recording && (
            <span
              className="absolute top-1.5 right-1.5 flex items-center gap-1 rounded-full px-1.5 py-0.5"
              style={{ background: "rgba(178,69,59,0.85)" }}
            >
              <span className="rounded-full" style={{ width: 4, height: 4, background: "#fff" }} />
              <span style={{ fontFamily: F_MONO, fontSize: 6.5, color: "#fff" }}>REC</span>
            </span>
          )}
        </div>
        <span style={{ fontFamily: F_MONO, fontSize: 9, color: C.textDim, marginTop: 6 }}>Pixel 8 Pro · 1080×2400</span>
      </div>

      <div className="flex flex-col gap-2 flex-1 min-w-0 justify-center">
        <button
          onClick={takeScreenshot}
          className="flex items-center gap-2 rounded-md px-2.5 py-1.5"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <Camera size={12} color={C.textMid} />
          <span style={{ fontFamily: F_UI, fontSize: 11, color: C.text }}>Screenshot</span>
          <span style={{ fontFamily: F_MONO, fontSize: 9, color: C.textDim, marginLeft: "auto" }}>
            → project assets
          </span>
        </button>
        <button
          onClick={() => setRecording((r) => !r)}
          className="flex items-center gap-2 rounded-md px-2.5 py-1.5"
          style={{
            background: recording ? C.redBg : C.s3,
            border: `1px solid ${recording ? "rgba(178,69,59,0.4)" : C.border}`,
          }}
        >
          <Video size={12} color={recording ? C.red : C.textMid} />
          <span style={{ fontFamily: F_UI, fontSize: 11, color: recording ? C.red : C.text }}>
            {recording ? "Stop recording" : "Record screen"}
          </span>
          <span style={{ fontFamily: F_MONO, fontSize: 9, color: C.textDim, marginLeft: "auto" }}>
            {recording ? "0:04" : "adb pull on stop"}
          </span>
        </button>
        <button
          className="flex items-center gap-2 rounded-md px-2.5 py-1.5"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <RotateCw size={12} color={C.textMid} />
          <span style={{ fontFamily: F_UI, fontSize: 11, color: C.text }}>Rotate</span>
        </button>
        <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, lineHeight: 1.6, marginTop: 4 }}>
          {lastTap ? (
            <>
              Last tap sent:{" "}
              <span style={{ fontFamily: F_MONO, color: C.teal }}>
                input tap {lastTap.x} {lastTap.y}
              </span>
            </>
          ) : (
            "Click the mirror to send input tap x y over ADB."
          )}
        </p>
      </div>
    </div>
  );
}

// §72.2: a new Devices tab, not a bespoke standalone panel — same
// hide/show and dock treatment every other panel gets (§62.2).
const SERIAL_LOG_SEED = [
  { t: "0.001", text: "boot: ESP32 rev 3, FreeRTOS v4.4.2" },
  { t: "0.048", text: 'wifi: connecting to "spartan-lab"...' },
  { t: "1.203", text: "wifi: connected, ip=192.168.1.42" },
  { t: "1.240", text: "mqtt: connecting to mqtt://localhost:1883" },
  { t: "1.312", text: "mqtt: connected, subscribed to sensors/temp" },
];

function SerialMonitorTab() {
  const [log, setLog] = useState(SERIAL_LOG_SEED);
  const [input, setInput] = useState("");

  function sendLine() {
    if (!input.trim()) return;
    setLog((prev) => [...prev, { t: (Math.random() * 5 + 5).toFixed(3), text: `> ${input}`, sent: true }]);
    setInput("");
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-3 py-2" style={{ borderBottom: `1px solid ${C.border}` }}>
        <Radio size={12} color={C.teal} />
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>ESP32-DevKitC · 115200 baud</span>
        <span style={{ fontFamily: F_MONO, fontSize: 9, color: C.textDim, marginLeft: "auto" }}>§72.2</span>
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2" style={{ fontFamily: F_MONO, fontSize: 11, lineHeight: 1.7 }}>
        {log.map((l, i) => (
          <div key={i} className="flex gap-2">
            <span style={{ color: C.textDim, flexShrink: 0, width: 48, textAlign: "right" }}>{l.t}s</span>
            <span style={{ color: l.sent ? C.accent : C.textMid }}>{l.text}</span>
          </div>
        ))}
      </div>
      <div className="px-3 py-2" style={{ borderTop: `1px solid ${C.border}` }}>
        <div
          className="flex items-center gap-2 rounded-md px-2.5 py-1.5"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && sendLine()}
            placeholder="Send a line to the device…"
            className="flex-1 bg-transparent outline-none min-w-0"
            style={{ fontFamily: F_MONO, fontSize: 11, color: C.text }}
          />
          <Send size={13} color={C.accent} className="cursor-pointer flex-shrink-0" onClick={sendLine} />
        </div>
      </div>
    </div>
  );
}

function DevicesPanel({ tab, setTab }) {
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1 px-3 pt-2" style={{ borderBottom: `1px solid ${C.border}` }}>
        {["devices", "logcat", "shell", "screen", "serial"].map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className="rounded-t-md px-2.5 py-1.5 capitalize"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: tab === t ? C.s3 : "transparent",
              color: tab === t ? C.text : C.textDim,
            }}
          >
            {t}
          </button>
        ))}
        <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim, marginLeft: "auto", marginBottom: 4 }}>
          4 of 8 ADB tabs · + Serial (§72.2)
        </span>
      </div>
      {tab === "screen" && <ScreenMirrorTab />}
      {tab === "serial" && <SerialMonitorTab />}
      {tab === "devices" && (
        <div className="p-3 space-y-1.5 overflow-y-auto flex-1 min-h-0">
          <div
            className="flex items-center justify-between rounded-md px-2.5 py-2"
            style={{ background: C.s3, border: `1px solid ${C.border}` }}
          >
            <div className="flex items-center gap-2">
              <span className="rounded-full" style={{ width: 6, height: 6, background: C.green }} />
              <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>
                Pixel 8 Pro · emulator-5554 · Android 15
              </span>
              <span
                className="rounded px-1 py-0.5"
                style={{ background: C.s2, fontSize: 8.5, color: C.textDim, fontFamily: F_MONO }}
              >
                emulator
              </span>
            </div>
            <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.accent, cursor: "pointer" }}>Attach JDWP</span>
          </div>
          {/* §61.2: WSA is a third device kind, same tabs/tools as any other
              connected device — labeled distinctly, not presented identically
              to a standard emulator, since its virtualization is Microsoft's
              to tune, not Spartan's. */}
          <div
            className="flex items-center justify-between rounded-md px-2.5 py-2"
            style={{ background: C.s3, border: `1px solid ${C.border}` }}
          >
            <div className="flex items-center gap-2">
              <span className="rounded-full" style={{ width: 6, height: 6, background: C.green }} />
              <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>
                Windows Subsystem for Android · localhost:58526
              </span>
              <span
                className="rounded px-1 py-0.5"
                style={{ background: C.tealBg, fontSize: 8.5, color: C.teal, fontFamily: F_MONO }}
              >
                WSA · §61.2
              </span>
            </div>
            <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.accent, cursor: "pointer" }}>Attach JDWP</span>
          </div>
        </div>
      )}
      {tab === "logcat" && (
        <div className="p-2 overflow-y-auto flex-1">
          {LOGCAT_LINES.map((l, i) => (
            <div key={i} className="flex items-center gap-2 px-1 py-0.5">
              <span
                style={{
                  fontFamily: F_MONO,
                  fontSize: 10,
                  color: l.level === "E" ? C.red : l.level === "D" ? C.textDim : C.teal,
                  width: 10,
                }}
              >
                {l.level}
              </span>
              <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim, width: 90 }}>{l.tag}</span>
              <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.text }}>{l.msg}</span>
            </div>
          ))}
        </div>
      )}
      {tab === "shell" && (
        <div className="p-3 flex-1">
          <div style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>
            $ <span style={{ color: C.text }}>adb shell pm list packages | grep spartan</span>
          </div>
          <div style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim, marginTop: 4 }}>
            package:com.spartan.demo
          </div>
        </div>
      )}
    </div>
  );
}

// §56.1 Source Control (local git) + §56.3 GitHub Pull Requests — one panel,
// two tabs, sharing the same diff-card visual language as an agent diff
// (§8.5) so a manual edit and an AI edit review identically, on purpose.
const GIT_CHANGED_FILES = [
  { path: "src/auth/middleware.rs", status: "M", add: 42, del: 18 },
  { path: "src/auth/routes.rs", status: "M", add: 27, del: 9 },
  { path: "src/auth/session_store.rs", status: "M", add: 6, del: 31 },
  { path: "src/auth/jwt_rotation.rs", status: "A", add: 58, del: 0 },
];
const GIT_CHECKS = [
  { name: "build", state: "pass" },
  { name: "test (unit)", state: "pass" },
  { name: "test (integration)", state: "fail" },
  { name: "lint", state: "pending" },
];
const CHECK_COLOR = { pass: "#4E9E72", fail: "#B2453B", pending: "#C99A3D" };

function GitPanel() {
  const [tab, setTab] = useState("changes");
  const [staged, setStaged] = useState(new Set(["src/auth/middleware.rs", "src/auth/routes.rs"]));
  const [message, setMessage] = useState("");
  const [committed, setCommitted] = useState(false);

  function toggleStage(path) {
    setStaged((prev) => {
      const next = new Set(prev);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });
  }
  function doCommit() {
    if (!message.trim() || staged.size === 0) return;
    setCommitted(true);
    setTimeout(() => setCommitted(false), 1800);
    setStaged(new Set());
    setMessage("");
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-3 px-3 pt-2" style={{ borderBottom: `1px solid ${C.border}` }}>
        {[
          ["changes", "Changes"],
          ["pr", "Pull Request"],
        ].map(([id, label]) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className="rounded-t-md px-2.5 py-1.5"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: tab === id ? C.s3 : "transparent",
              color: tab === id ? C.text : C.textDim,
            }}
          >
            {label}
          </button>
        ))}
        <span
          className="flex items-center gap-1"
          style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim, marginLeft: "auto", marginBottom: 4 }}
        >
          <GitBranch size={10} /> auth/jwt-refactor → main
        </span>
      </div>

      {tab === "changes" && (
        <div className="flex-1 flex gap-3 p-3 min-h-0">
          <div className="flex-1 overflow-y-auto space-y-1">
            {GIT_CHANGED_FILES.map((f) => {
              const isStaged = staged.has(f.path);
              return (
                <div
                  key={f.path}
                  className="flex items-center gap-2 rounded-md px-2 py-1.5"
                  style={{ background: C.s3, border: `1px solid ${C.border}` }}
                >
                  <input
                    type="checkbox"
                    checked={isStaged}
                    onChange={() => toggleStage(f.path)}
                    className="cursor-pointer"
                    style={{ accentColor: C.accent }}
                  />
                  <span
                    className="rounded px-1"
                    style={{
                      fontFamily: F_MONO,
                      fontSize: 9,
                      color: f.status === "A" ? C.green : C.amber,
                      background: C.s2,
                    }}
                  >
                    {f.status}
                  </span>
                  <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.text, flex: 1 }}>{f.path}</span>
                  <span style={{ fontFamily: F_MONO, fontSize: 9.5 }}>
                    <span style={{ color: C.green }}>+{f.add}</span> <span style={{ color: C.red }}>-{f.del}</span>
                  </span>
                </div>
              );
            })}
          </div>
          <div className="flex flex-col gap-2" style={{ width: 220 }}>
            <textarea
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="fix: rotate JWTs on refresh, keep session compat"
              className="flex-1 rounded-md px-2.5 py-2 resize-none outline-none"
              style={{
                background: C.s2,
                border: `1px solid ${C.border}`,
                fontFamily: F_MONO,
                fontSize: 11,
                color: C.text,
              }}
            />
            <button
              onClick={doCommit}
              className="rounded-md py-1.5"
              style={{ background: C.accent, fontFamily: F_UI, fontSize: 11, color: "#fff" }}
            >
              Commit {staged.size} file{staged.size === 1 ? "" : "s"}
            </button>
            {committed && (
              <span
                className="flex items-center gap-1 fade-in-up"
                style={{ color: C.green, fontFamily: F_MONO, fontSize: 10.5 }}
              >
                <CheckCircle2 size={11} /> Committed
              </span>
            )}
          </div>
        </div>
      )}

      {tab === "pr" && (
        <div className="p-3 overflow-y-auto flex-1">
          <div className="flex items-center justify-between mb-2">
            <span style={{ fontFamily: F_UI, fontSize: 12, color: C.text }}>#412 Refactor session auth to JWT</span>
            <span
              className="rounded px-1.5 py-0.5"
              style={{ background: C.amberBg, color: C.amber, fontSize: 9.5, fontFamily: F_MONO }}
            >
              2 changes requested
            </span>
          </div>
          <div className="flex gap-1 mb-3">
            {GIT_CHECKS.map((c) => (
              <div
                key={c.name}
                className="flex-1 rounded px-1.5 py-1"
                style={{ background: C.s3, border: `1px solid ${C.border}` }}
                title={c.name}
              >
                <div className="rounded-full mb-1" style={{ height: 3, background: CHECK_COLOR[c.state] }} />
                <span style={{ fontFamily: F_MONO, fontSize: 9, color: C.textDim }}>{c.name}</span>
              </div>
            ))}
          </div>
          <div
            className="rounded-md px-2.5 py-2 mb-2"
            style={{ background: C.accentBg, border: `1px solid ${C.accentBorder}` }}
          >
            <div className="flex items-center gap-1.5 mb-1">
              <Sparkles size={11} color={C.accent} />
              <span style={{ fontFamily: F_UI, fontSize: 11, color: C.accent, fontWeight: 600 }}>
                Leo self-review posted
              </span>
            </div>
            <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textMid, lineHeight: 1.5 }}>
              Flagged one risk: <span style={{ color: C.text }}>session_store.rs</span> loses its retry-on-timeout
              wrapper — posted as a real PR comment thread, not just visible in Spartan (§15, §56.3).
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim }}>Linked issue</span>
            <span
              className="rounded px-1.5 py-0.5"
              style={{ background: C.s3, color: C.teal, fontSize: 10, fontFamily: F_MONO }}
            >
              AUTH-142
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

// §59: a real embedded terminal, multi-tab, PTY-backed in the real product.
// §59.2's distinction matters here: this is the human's own unsandboxed
// shell (like any OS terminal), not Leo's approval-gated run_terminal tool
// (§4.5) — nothing here is approval-gated, on purpose.
const TERMINAL_TAB_DEFS = [
  {
    id: "zsh",
    label: "zsh",
    kind: "local",
    initial: [
      { t: "cmd", text: "$ cargo test --workspace --release" },
      { t: "out", text: "   Compiling spartan-core v0.1.0" },
      { t: "ok", text: "test result: ok. 14 passed; 0 failed" },
    ],
  },
  {
    id: "wsl",
    label: "WSL: Ubuntu-22.04",
    kind: "wsl",
    initial: [
      { t: "cmd", text: "$ uname -a" },
      { t: "out", text: "Linux ubuntu 5.15.167.4-microsoft-standard-WSL2 x86_64 GNU/Linux" },
    ],
  },
];

// §65: a real, driveable browser panel — not a screenshot-on-a-timer.
// Click-to-inspect feeds selector/style context to Leo the same way a log
// line or stack trace does elsewhere (§14); recorded interactions become a
// real, reviewable Playwright script (§65.1), never auto-saved silently.
const PAGE_ELEMENTS = [
  { id: "heading", label: "Checkout", tag: "h2", selector: "h2.checkout-title", top: 14, height: 24 },
  { id: "email", label: "Email address", tag: "input", selector: 'input[name="email"]', top: 50, height: 26 },
  { id: "card", label: "Card number", tag: "input", selector: 'input[name="card_number"]', top: 88, height: 26 },
  { id: "submit", label: "Pay $48.00", tag: "button", selector: 'button[type="submit"]', top: 130, height: 30 },
];

function PlaywrightPanel() {
  const [selected, setSelected] = useState(null);
  const [recording, setRecording] = useState(false);
  const [recordedSteps, setRecordedSteps] = useState([]);
  // §67: parity with Antigravity's Browser Subagent — Leo can drive this
  // same panel autonomously as part of its own verification loop (§4, same
  // pattern §21.6 already uses for Android compose-preview renders), not
  // just the user manually clicking around.
  const [leoDriving, setLeoDriving] = useState(false);

  function clickElement(el) {
    setSelected(el);
    if (recording) {
      const line = el.tag === "input" ? `await page.click('${el.selector}');` : `await page.click('${el.selector}');`;
      setRecordedSteps((prev) => [...prev, line]);
    }
  }

  function toggleLeoDriving() {
    setLeoDriving((d) => {
      const next = !d;
      if (next) {
        const submit = PAGE_ELEMENTS.find((e) => e.id === "submit");
        setSelected(submit);
        setRecordedSteps((prev) => [
          ...prev,
          `// Leo: clicking through checkout to verify the JWT refactor`,
          `await page.click('${submit.selector}');`,
        ]);
      }
      return next;
    });
  }

  return (
    <div className="flex h-full">
      <div className="flex-1 flex flex-col min-w-0" style={{ borderRight: `1px solid ${C.border}` }}>
        <div
          className="flex items-center gap-2 px-3 py-2 overflow-hidden"
          style={{ borderBottom: `1px solid ${C.border}` }}
        >
          <div className="flex items-center gap-1 flex-shrink-0">
            <span className="rounded-full" style={{ width: 8, height: 8, background: "#3a3a3a" }} />
            <span className="rounded-full" style={{ width: 8, height: 8, background: "#3a3a3a" }} />
          </div>
          <span
            className="rounded px-2 py-1 flex-1 min-w-0 truncate"
            style={{ background: C.s3, fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}
          >
            localhost:3000/checkout
          </span>
          <button
            onClick={toggleLeoDriving}
            title={
              leoDriving
                ? "Leo is driving — click to take back manual control"
                : "Let Leo drive (§67 — Browser Subagent)"
            }
            className="flex items-center justify-center rounded px-1.5 py-1 flex-shrink-0"
            style={{ background: leoDriving ? C.accentBg : C.s3, color: leoDriving ? C.accent : C.textDim }}
          >
            <Sparkles size={12} />
          </button>
          <button
            onClick={() => setRecording((r) => !r)}
            title={recording ? "Recording — click to stop" : "Record"}
            className="flex items-center justify-center rounded px-1.5 py-1 flex-shrink-0"
            style={{ background: recording ? C.redBg : C.s3, color: recording ? C.red : C.textDim }}
          >
            <span className="rounded-full" style={{ width: 6, height: 6, background: recording ? C.red : C.textDim }} />
          </button>
          <button
            title="View trace"
            className="rounded px-1.5 py-1 flex items-center flex-shrink-0"
            style={{ background: C.s3, color: C.textDim }}
          >
            <History size={12} />
          </button>
        </div>
        {leoDriving && (
          <div
            className="flex items-center gap-1.5 px-3 py-1 fade-in-up"
            style={{ background: C.accentBg, borderBottom: `1px solid ${C.accentBorder}` }}
          >
            <span className="rounded-full" style={{ width: 5, height: 5, background: C.accent }} />
            <span style={{ fontFamily: F_UI, fontSize: 10, color: C.accent }}>
              Leo is driving — verifying the checkout flow itself
            </span>
          </div>
        )}
        <div className="flex-1 relative p-4" style={{ background: "#0d0d10" }}>
          <div
            className="relative rounded-lg mx-auto"
            style={{ width: 260, height: 170, background: "#141416", border: `1px solid ${C.borderLt}` }}
          >
            {PAGE_ELEMENTS.map((el) => (
              <div
                key={el.id}
                onClick={() => clickElement(el)}
                className="absolute left-3 right-3 rounded cursor-pointer flex items-center px-2"
                style={{
                  top: el.top,
                  height: el.height,
                  background: el.tag === "button" ? C.accentBg : C.s3,
                  border: `1px solid ${selected?.id === el.id ? C.accent : el.tag === "button" ? C.accentBorder : C.border}`,
                }}
              >
                <span
                  style={{
                    fontFamily: F_UI,
                    fontSize: el.tag === "h2" ? 12 : 10.5,
                    fontWeight: el.tag === "h2" ? 600 : 400,
                    color: el.tag === "button" ? C.accent : C.textMid,
                  }}
                >
                  {el.label}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
      <div className="flex flex-col flex-shrink-0" style={{ width: 200 }}>
        <div className="px-3 py-2" style={{ borderBottom: `1px solid ${C.border}` }}>
          <span
            style={{ fontFamily: F_UI, fontSize: 10.5, fontWeight: 600, color: C.textDim, letterSpacing: "0.05em" }}
          >
            INSPECTOR
          </span>
        </div>
        <div className="p-3" style={{ borderBottom: `1px solid ${C.border}` }}>
          {selected ? (
            <div>
              <div style={{ fontFamily: F_MONO, fontSize: 11, color: C.teal, marginBottom: 4 }}>
                {selected.selector}
              </div>
              <div style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>
                role: {selected.tag === "button" ? "button" : selected.tag === "input" ? "textbox" : "heading"}
              </div>
              <div style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>
                accessible name: "{selected.label}"
              </div>
            </div>
          ) : (
            <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim }}>
              Click any element in the live page (§65.1).
            </span>
          )}
        </div>
        <div className="p-3 flex-1 overflow-y-auto">
          <div style={{ fontFamily: F_UI, fontSize: 10.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
            RECORDED TEST
          </div>
          {recordedSteps.length === 0 ? (
            <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>
              Toggle Record, then click around — steps become a real, reviewable Playwright script.
            </span>
          ) : (
            <pre style={{ fontFamily: F_MONO, fontSize: 10, color: C.text, whiteSpace: "pre-wrap", margin: 0 }}>
              {recordedSteps.join("\n")}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}

function TerminalPanel() {
  const [activeTab, setActiveTab] = useState("zsh");
  const [tabs, setTabs] = useState(() => Object.fromEntries(TERMINAL_TAB_DEFS.map((t) => [t.id, t.initial])));
  const [input, setInput] = useState("");

  const isCoPilotQuery = input.trim().startsWith("#");
  const suggestion = isCoPilotQuery
    ? { command: "find . -name '*.rs' -newer .git/HEAD~7 -mtime -7 | xargs grep -l TODO", destructive: false }
    : null;

  function runLine(text) {
    setTabs((prev) => ({
      ...prev,
      [activeTab]: [
        ...prev[activeTab],
        { t: "cmd", text: `$ ${text}` },
        { t: "out", text: "(simulated) command queued — no real subprocess in this reference mockup" },
      ],
    }));
    setInput("");
  }
  function acceptSuggestion() {
    setInput(suggestion.command);
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1 px-3 pt-2" style={{ borderBottom: `1px solid ${C.border}` }}>
        {TERMINAL_TAB_DEFS.map((t) => (
          <button
            key={t.id}
            onClick={() => setActiveTab(t.id)}
            className="rounded-t-md px-2.5 py-1.5 flex items-center gap-1.5"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: activeTab === t.id ? C.s3 : "transparent",
              color: activeTab === t.id ? C.text : C.textDim,
            }}
          >
            {t.kind === "wsl" ? (
              <Monitor size={11} color={activeTab === t.id ? C.teal : C.textDim} />
            ) : (
              <TerminalSquare size={11} />
            )}
            {t.label}
          </button>
        ))}
        <span style={{ fontFamily: F_MONO, fontSize: 9, color: C.textDim, marginLeft: "auto", marginBottom: 4 }}>
          your own shell, not sandboxed · §59.2
        </span>
      </div>
      <div className="flex-1 overflow-y-auto px-3 py-2" style={{ fontFamily: F_MONO, fontSize: 11.5, lineHeight: 1.7 }}>
        {tabs[activeTab].map((line, i) => (
          <div key={i} style={{ color: line.t === "cmd" ? C.text : line.t === "ok" ? C.green : C.textDim }}>
            {line.text}
          </div>
        ))}
      </div>
      <div className="px-3 py-2" style={{ borderTop: `1px solid ${C.border}` }}>
        {suggestion && (
          <div
            className="rounded-md px-2.5 py-2 mb-2 fade-in-up"
            style={{ background: C.s2, border: `1px dashed ${C.borderLt}` }}
          >
            <div className="flex items-center gap-1.5 mb-1">
              <Sparkles size={11} color={C.teal} />
              <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.teal }}>Co-pilot suggestion</span>
            </div>
            <div style={{ fontFamily: F_MONO, fontSize: 11, color: C.text, marginBottom: 6 }}>{suggestion.command}</div>
            <div className="flex gap-1.5">
              <button
                onClick={acceptSuggestion}
                className="rounded px-2 py-1"
                style={{ background: C.tealBg, color: C.teal, fontSize: 10, fontFamily: F_UI }}
              >
                Insert
              </button>
              <button
                className="rounded px-2 py-1"
                style={{ background: C.s3, color: C.textDim, fontSize: 10, fontFamily: F_UI }}
              >
                Dry run (§59.3)
              </button>
            </div>
          </div>
        )}
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && input.trim() && !isCoPilotQuery) runLine(input);
          }}
          placeholder="type a command, or # describe what you want in plain language"
          className="w-full rounded-md px-2.5 py-1.5 outline-none"
          style={{
            background: C.s2,
            border: `1px solid ${C.border}`,
            fontFamily: F_MONO,
            fontSize: 11.5,
            color: C.text,
          }}
        />
      </div>
    </div>
  );
}

function EditorView({ provider, panelVisibility = {}, onTogglePanel }) {
  const [dockOpen, setDockOpen] = useState(true);
  const [breakpoints, setBreakpoints] = useState(new Set([44]));
  const [bottomTab, setBottomTab] = useState(null); // null | "debug" | "devices" | "git" | "terminal"
  const [devicesTab, setDevicesTab] = useState("devices");
  const [hit, setHit] = useState(false);
  // §71: Antigravity draws a real line between two chat surfaces — a side
  // panel for conversations and broad changes, and inline chat (Cmd+I) that
  // operates on the exact code you're looking at, not a copy-pasted
  // fragment of it. This is the inline half; AgentView (§8) is the side half.
  const [selectedLine, setSelectedLine] = useState(41);
  const [inlineChatOpen, setInlineChatOpen] = useState(false);
  const [inlineChatInput, setInlineChatInput] = useState("");

  useEffect(() => {
    function onKeyDown(e) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "i") {
        e.preventDefault();
        setInlineChatOpen((o) => !o);
      } else if (e.key === "Escape") {
        setInlineChatOpen(false);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  // The Screen mirror needs real vertical room to read as a phone, not a
  // strip — the dock grows for it the same way a real IDE lets this one
  // panel type resize, rather than cropping a phone aspect into 128px.
  // Source Control and Terminal get a similar bump: a file list/PR summary
  // or a real shell transcript doesn't fit legibly in 128px either.
  const dockHeight =
    bottomTab === "devices" && devicesTab === "screen"
      ? 340
      : bottomTab === "playwright"
        ? 380
        : bottomTab === "git" || bottomTab === "terminal"
          ? 300
          : 160;

  function toggleBreakpoint(n) {
    setBreakpoints((prev) => {
      const next = new Set(prev);
      next.has(n) ? next.delete(n) : next.add(n);
      return next;
    });
  }
  function runToBreakpoint() {
    setBottomTab("debug");
    setHit(true);
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex flex-1 min-h-0">
        {panelVisibility.fileTree !== false && (
          <div
            className="w-48 flex-shrink-0 overflow-y-auto py-2 flex flex-col"
            style={{ borderRight: `1px solid ${C.border}` }}
          >
            <div className="flex items-center justify-between px-2 pb-1.5">
              <span
                style={{ fontFamily: F_UI, fontSize: 10.5, fontWeight: 600, color: C.textDim, letterSpacing: "0.06em" }}
              >
                FILES
              </span>
              {onTogglePanel && (
                <button
                  onClick={() => onTogglePanel("fileTree")}
                  title="Hide panel — bring back via View (§62.2)"
                  style={{ color: C.textDim }}
                >
                  <EyeOff size={12} />
                </button>
              )}
            </div>
            {FILES.map((f, i) => (
              <div
                key={i}
                className="flex items-center gap-1.5 px-2 py-1 cursor-pointer"
                style={{ paddingLeft: 10 + (f.indent || 0) * 14, background: f.active ? C.s3 : "transparent" }}
              >
                {f.type === "folder" ? (
                  f.open ? (
                    <FolderOpen size={13} color={C.textDim} />
                  ) : (
                    <Folder size={13} color={C.textDim} />
                  )
                ) : (
                  <FileCode size={13} color={f.active ? C.accent : C.textDim} />
                )}
                <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: f.active ? C.text : C.textMid }}>
                  {f.name}
                </span>
                {f.tick === "accent" && (
                  <span className="ml-auto rounded-full" style={{ width: 5, height: 5, background: C.accent }} />
                )}
                {f.tick === "diag" && (
                  <span className="ml-auto rounded-full" style={{ width: 5, height: 5, background: C.amber }} />
                )}
              </div>
            ))}
          </div>
        )}

        <div className="flex-1 flex flex-col min-w-0">
          <div
            className="flex items-center gap-2 px-4 py-2 min-w-0"
            style={{ borderBottom: `1px solid ${C.border}`, background: C.s2 }}
          >
            <FileCode size={12} color={C.accent} className="flex-shrink-0" />
            <span className="flex-shrink-0" style={{ fontFamily: F_MONO, fontSize: 12, color: C.text }}>
              middleware.rs
            </span>
            <span
              className="rounded px-1.5 py-0.5 ml-1 flex-shrink-0"
              style={{ background: C.accentBg, color: C.accent, fontSize: 9.5, fontFamily: F_MONO }}
              title="AI-edited"
            >
              AI
            </span>
            <span
              className="rounded px-1.5 py-0.5 flex items-center gap-1 min-w-0 truncate flex-shrink"
              style={{ background: C.s3, color: C.textDim, fontSize: 9.5, fontFamily: F_MONO, marginLeft: 6 }}
              title="AUTH-142 · 3 tests · deployed 2h ago"
            >
              <GitBranch size={9} className="flex-shrink-0" /> AUTH-142
            </span>
            <span
              className="rounded px-1.5 py-0.5 flex-shrink-0"
              style={{ background: C.s3, color: C.textDim, fontSize: 9.5, fontFamily: F_MONO }}
              title="Inline chat, scoped to whatever line you're on — distinct from the side chat (§71)"
            >
              ⌘I
            </span>
            <button
              onClick={runToBreakpoint}
              className="ml-auto rounded px-2 py-1 flex items-center gap-1 flex-shrink-0"
              style={{ background: C.accentBg, color: C.accent, fontSize: 10.5, fontFamily: F_UI }}
            >
              ▶ Run
            </button>
            {/* §62.2-style badge cluster: icon-only, one connected group, so
                a growing set of dock panels never needs horizontal scroll
                (fixed the real overflow this caused when it was 4 labeled
                buttons in a row) — a title tooltip carries the label instead. */}
            <div
              className="flex items-center gap-0.5 rounded-md p-0.5 flex-shrink-0"
              style={{ background: C.s3, border: `1px solid ${C.border}` }}
            >
              {[
                { id: "git", icon: GitBranch, label: "Source Control (§56)" },
                { id: "devices", icon: Smartphone, label: "Devices (§33)" },
                { id: "terminal", icon: TerminalSquare, label: "Terminal (§59)" },
                { id: "playwright", icon: MonitorPlay, label: "Live Browser — Playwright (§65)" },
              ].map((b) => {
                const Icon = b.icon;
                const active = bottomTab === b.id;
                return (
                  <button
                    key={b.id}
                    onClick={() => setBottomTab(active ? null : b.id)}
                    title={b.label}
                    className="rounded px-1.5 py-1"
                    style={{ background: active ? C.accentBg : "transparent", color: active ? C.accent : C.textDim }}
                  >
                    <Icon size={12} />
                  </button>
                );
              })}
            </div>
          </div>
          <div className="flex-1 overflow-y-auto py-3" style={{ background: C.bg }}>
            {CODE_LINES.map((l) => {
              const bp = breakpoints.has(l.n);
              const selected = selectedLine === l.n;
              return (
                <React.Fragment key={l.n}>
                  <div
                    onClick={() => setSelectedLine(l.n)}
                    className="flex items-center px-4 group cursor-text"
                    style={{
                      background: hit && bp ? C.accentBg : l.diag ? C.redBg : selected ? C.s2 : "transparent",
                    }}
                  >
                    <span
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleBreakpoint(l.n);
                      }}
                      className="cursor-pointer flex-shrink-0 rounded-full"
                      style={{
                        width: 8,
                        height: 8,
                        marginRight: 6,
                        background: bp ? C.accent : "transparent",
                        border: bp ? "none" : `1px solid transparent`,
                      }}
                    />
                    <span
                      style={{
                        fontFamily: F_MONO,
                        fontSize: 11,
                        color: C.textDim,
                        width: 22,
                        textAlign: "right",
                        marginRight: 12,
                      }}
                    >
                      {l.n}
                    </span>
                    <span
                      className="flex-shrink-0"
                      style={{
                        width: 3,
                        height: 14,
                        marginRight: 8,
                        background: l.tick === "accent" ? C.accent : "transparent",
                        borderRadius: 2,
                      }}
                    />
                    <span style={{ fontFamily: F_MONO, fontSize: 12.5, color: C.text, whiteSpace: "pre" }}>{l.t}</span>
                    {l.diag && <AlertTriangle size={11} color={C.amber} className="ml-2 flex-shrink-0" />}
                  </div>
                  {selected && inlineChatOpen && (
                    <div className="px-4 py-2 fade-in-up" style={{ background: C.s1 }}>
                      <div
                        className="rounded-lg px-3 py-2"
                        style={{ background: C.s2, border: `1px solid ${C.accentBorder}` }}
                      >
                        <div className="flex items-center gap-1.5 mb-1.5">
                          <Sparkles size={11} color={C.accent} />
                          <span style={{ fontFamily: F_UI, fontSize: 10, color: C.textDim }}>
                            Inline chat · scoped to line {l.n} · Esc to close
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          <input
                            autoFocus
                            value={inlineChatInput}
                            onChange={(e) => setInlineChatInput(e.target.value)}
                            placeholder="e.g. add a doc comment explaining this check"
                            className="flex-1 bg-transparent outline-none min-w-0"
                            style={{ fontFamily: F_UI, fontSize: 12, color: C.text }}
                          />
                          <Send size={13} color={C.accent} className="cursor-pointer flex-shrink-0" />
                        </div>
                      </div>
                    </div>
                  )}
                </React.Fragment>
              );
            })}
          </div>
          {bottomTab && (
            <div
              className="fade-in-up"
              style={{
                height: dockHeight,
                transition: "height 160ms ease",
                borderTop: `1px solid ${C.border}`,
                background: C.s1,
              }}
            >
              <div
                className="flex items-center justify-between px-3 py-1.5"
                style={{ borderBottom: `1px solid ${C.border}` }}
              >
                <span
                  style={{
                    fontFamily: F_UI,
                    fontSize: 10.5,
                    fontWeight: 600,
                    color: C.textDim,
                    letterSpacing: "0.05em",
                  }}
                >
                  {bottomTab === "debug"
                    ? "DEBUG CONSOLE"
                    : bottomTab === "git"
                      ? "SOURCE CONTROL · §56"
                      : bottomTab === "terminal"
                        ? "TERMINAL · §59"
                        : bottomTab === "playwright"
                          ? "LIVE BROWSER · §65"
                          : "DEVICE PANEL"}
                </span>
                <button onClick={() => setBottomTab(null)}>
                  <X size={12} color={C.textDim} />
                </button>
              </div>
              <div style={{ height: dockHeight - 32 }}>
                {bottomTab === "debug" ? (
                  <DebugConsole hit={hit} />
                ) : bottomTab === "git" ? (
                  <GitPanel />
                ) : bottomTab === "terminal" ? (
                  <TerminalPanel />
                ) : bottomTab === "playwright" ? (
                  <PlaywrightPanel />
                ) : (
                  <DevicesPanel tab={devicesTab} setTab={setDevicesTab} />
                )}
              </div>
            </div>
          )}
        </div>

        {dockOpen && (
          <div
            className="w-64 flex-shrink-0 flex flex-col"
            style={{ borderLeft: `1px solid ${C.border}`, background: C.s1 }}
          >
            <div
              className="flex items-center justify-between px-3 py-2.5"
              style={{ borderBottom: `1px solid ${C.border}` }}
            >
              <ModelBadge provider={provider} />
              <button onClick={() => setDockOpen(false)}>
                <X size={13} color={C.textDim} />
              </button>
            </div>
            <div className="flex-1 px-3 py-3">
              <p style={{ fontFamily: F_UI, fontSize: 12, color: C.textMid, lineHeight: 1.6 }}>
                Docked Leo — same agent, compact view. Open full Agent View for the plan and artifact history.
              </p>
            </div>
            <div className="px-3 py-3" style={{ borderTop: `1px solid ${C.border}` }}>
              <div className="rounded-lg px-2.5 py-2" style={{ background: C.s2, border: `1px solid ${C.border}` }}>
                <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textDim }}>Ask about this file…</span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/* ---------- design view ---------- */
function DesignView() {
  const [dMode, setDMode] = useState("canvas");
  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center justify-between px-5 py-3" style={{ borderBottom: `1px solid ${C.border}` }}>
        <div
          className="flex items-center gap-0.5 rounded-lg p-0.5"
          style={{ background: C.s2, border: `1px solid ${C.border}` }}
        >
          {[
            { id: "canvas", label: "Hands-on Canvas", icon: ComponentIcon },
            { id: "agent", label: "Agent-Driven Generation", icon: Wand2 },
          ].map((m) => {
            const Icon = m.icon;
            const active = dMode === m.id;
            return (
              <button
                key={m.id}
                onClick={() => setDMode(m.id)}
                className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs"
                style={{
                  fontFamily: F_UI,
                  background: active ? C.accentBg : "transparent",
                  color: active ? C.accent : C.textMid,
                  border: `1px solid ${active ? C.accentBorder : "transparent"}`,
                }}
              >
                <Icon size={12} />
                {m.label}
              </button>
            );
          })}
        </div>
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>DESIGN.md · spartan-brand</span>
      </div>

      {dMode === "canvas" ? (
        <div className="flex-1 flex items-center justify-center p-8" style={{ background: "#08080A" }}>
          <div className="rounded-xl p-5 w-72" style={{ background: C.s1, border: `1px solid ${C.borderLt}` }}>
            <div style={{ fontFamily: F_UI, fontWeight: 600, fontSize: 14, color: C.text }}>Pro plan</div>
            <div style={{ fontFamily: F_MONO, fontSize: 20, color: C.accent, marginTop: 4 }}>
              $24<span style={{ fontSize: 11, color: C.textDim }}>/mo</span>
            </div>
            <div className="mt-3 space-y-1.5">
              {["Unlimited Leo requests", "Full GUI builder", "MCP connectors"].map((f) => (
                <div key={f} className="flex items-center gap-1.5">
                  <Check size={11} color={C.green} />
                  <span style={{ fontFamily: F_UI, fontSize: 11, color: C.textMid }}>{f}</span>
                </div>
              ))}
            </div>
            <div
              className="mt-4 rounded-md text-center py-1.5"
              style={{ background: C.accent, fontFamily: F_UI, fontSize: 12, fontWeight: 600, color: "#fff" }}
            >
              Choose plan
            </div>
          </div>
        </div>
      ) : (
        <div className="flex-1 flex flex-col p-6">
          <div className="rounded-lg px-3.5 py-2.5 mb-4" style={{ background: C.s2, border: `1px solid ${C.border}` }}>
            <span style={{ fontFamily: F_UI, fontSize: 12.5, color: C.textMid }}>
              "Build a pricing page with three tiers, matching our brand"
            </span>
          </div>
          <div
            className="flex-1 rounded-lg flex items-center justify-center"
            style={{ background: C.s1, border: `1px dashed ${C.borderLt}` }}
          >
            <div className="flex flex-col items-center gap-2">
              <Wand2 size={20} color={C.accent} />
              <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>
                Streaming artifact into sandboxed preview…
              </span>
              <button
                className="mt-2 rounded-md px-3 py-1.5"
                style={{
                  background: C.accentBg,
                  border: `1px solid ${C.accentBorder}`,
                  fontFamily: F_UI,
                  fontSize: 11,
                  color: C.accent,
                }}
              >
                Promote to Hands-on Canvas
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/* ---------- auxiliary pane ---------- */
// §67: Antigravity's Artifacts are commentable the way a doc is — feedback
// left directly on the plan/diff/verification the agent produced, not a
// separate chat message about it. Every ArtifactShell gets this for free
// rather than bolting a comment thread onto each artifact type separately.
function ArtifactShell({ title, icon, children }) {
  const Icon = icon;
  const [commentsOpen, setCommentsOpen] = useState(false);
  const [comments, setComments] = useState([]);
  const [draft, setDraft] = useState("");

  function postComment() {
    if (!draft.trim()) return;
    setComments((c) => [...c, draft.trim()]);
    setDraft("");
  }

  return (
    <div className="rounded-lg fade-in-up" style={{ background: C.s2, border: `1px solid ${C.border}` }}>
      <div className="flex items-center gap-1.5 px-3 py-2" style={{ borderBottom: `1px solid ${C.border}` }}>
        <Icon size={12} color={C.textDim} />
        <span style={{ fontFamily: F_UI, fontSize: 11, fontWeight: 600, color: C.textMid, letterSpacing: "0.03em" }}>
          {title}
        </span>
        <button
          onClick={() => setCommentsOpen((o) => !o)}
          title="Comment on this artifact (§67)"
          className="ml-auto flex items-center gap-1"
          style={{ color: comments.length ? C.accent : C.textDim }}
        >
          <MessageSquare size={11} />
          {comments.length > 0 && <span style={{ fontFamily: F_MONO, fontSize: 9.5 }}>{comments.length}</span>}
        </button>
      </div>
      <div className="p-3">{children}</div>
      {commentsOpen && (
        <div className="px-3 pb-3 fade-in-up">
          {comments.map((c, i) => (
            <div
              key={i}
              className="rounded px-2 py-1.5 mb-1.5"
              style={{ background: C.s3, fontFamily: F_UI, fontSize: 11, color: C.text }}
            >
              {c}
            </div>
          ))}
          <div className="flex gap-1.5">
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && postComment()}
              placeholder="Leave feedback on this artifact…"
              className="flex-1 rounded px-2 py-1 min-w-0"
              style={{
                background: C.s3,
                border: `1px solid ${C.border}`,
                color: C.text,
                fontFamily: F_UI,
                fontSize: 11,
              }}
            />
            <button
              onClick={postComment}
              className="rounded px-2 flex-shrink-0"
              style={{ background: C.accentBg, color: C.accent, fontSize: 11, fontFamily: F_UI }}
            >
              Post
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// §52.4: Spartan supervises a Fleet session's process (path-jailed subprocess,
// same sandbox as §4.5) but cannot see or gate its internal tool-call
// approvals the way it does Leo's — so the Auxiliary Pane shows that
// reduced-trust posture plainly instead of fabricating plan/diff artifacts
// the session never actually produced.
function FleetAuxPane({ session, onHide }) {
  const content = SESSION_CONTENT[session.id];
  return (
    <div className="w-80 flex-shrink-0 flex flex-col" style={{ borderLeft: `1px solid ${C.border}`, background: C.s1 }}>
      <div className="flex items-center justify-between px-3 py-3" style={{ borderBottom: `1px solid ${C.border}` }}>
        <span style={{ fontFamily: F_UI, fontSize: 11, fontWeight: 600, color: C.textDim, letterSpacing: "0.06em" }}>
          AUXILIARY PANE
        </span>
        <div className="flex items-center gap-2">
          <FleetBadge engine={session.engine} />
          {onHide && (
            <button onClick={onHide} title="Hide (§62.2)">
              <EyeOff size={13} color={C.textDim} />
            </button>
          )}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <ArtifactShell title="Supervision Posture" icon={ShieldCheck}>
          <div className="flex items-start gap-2">
            <AlertTriangle size={13} color={C.amber} className="flex-shrink-0 mt-0.5" />
            <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid, lineHeight: 1.5 }}>
              Spartan sandboxes this process (path-jailed, no auto secret injection) but can't see
              {(ENGINE_META[session.engine] || {}).label || session.engine}'s internal tool-call approvals. No
              autonomous-approval tier applies — destructive actions still confirm here. §52.4
            </span>
          </div>
        </ArtifactShell>
        <ArtifactShell title="Usage (self-reported)" icon={Clock}>
          <div className="flex items-center gap-3 mb-1.5">
            <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.text }}>
              {content.usage.tokensSelfReported} tokens
            </span>
            <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>
              {content.usage.requests} requests
            </span>
          </div>
          <span
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5"
            style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim, background: C.s3 }}
          >
            parsed from CLI output, not precisely metered · §52.3
          </span>
        </ArtifactShell>
      </div>
    </div>
  );
}

function AuxPane({ session, provider, onHide }) {
  if (session.kind === "fleet") {
    return <FleetAuxPane session={session} onHide={onHide} />;
  }
  const content = SESSION_CONTENT[session.id];
  const [diffStatus, setDiffStatus] = useState({});
  const [planApproved, setPlanApproved] = useState(session.status !== "running");

  useEffect(() => {
    setDiffStatus({});
    setPlanApproved(session.status !== "running");
  }, [session.id]);

  const accept = (file) => setDiffStatus((s) => ({ ...s, [file]: "accepted" }));
  const reject = (file) => setDiffStatus((s) => ({ ...s, [file]: "rejected" }));

  return (
    <div className="w-80 flex-shrink-0 flex flex-col" style={{ borderLeft: `1px solid ${C.border}`, background: C.s1 }}>
      <div className="flex items-center justify-between px-3 py-3" style={{ borderBottom: `1px solid ${C.border}` }}>
        <span style={{ fontFamily: F_UI, fontSize: 11, fontWeight: 600, color: C.textDim, letterSpacing: "0.06em" }}>
          AUXILIARY PANE
        </span>
        <div className="flex items-center gap-2">
          <ModelBadge provider={provider} />
          {onHide && (
            <button onClick={onHide} title="Hide (§62.2)">
              <EyeOff size={13} color={C.textDim} />
            </button>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        <ArtifactShell title="Implementation Plan" icon={Sparkles}>
          <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 2 }}>{content.planTitle}</div>
          <div style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim, marginBottom: 10 }}>{content.scope}</div>
          {!planApproved ? (
            <div className="flex gap-1.5">
              <button
                onClick={() => setPlanApproved(true)}
                className="flex-1 rounded-md py-1.5 flex items-center justify-center gap-1"
                style={{ background: C.accent, fontFamily: F_UI, fontSize: 11, color: "#fff" }}
              >
                <Check size={11} /> Approve
              </button>
              <button
                className="rounded-md px-2.5"
                style={{ background: C.s3, border: `1px solid ${C.border}`, color: C.textMid }}
              >
                Edit
              </button>
              <button
                className="rounded-md px-2.5"
                style={{ background: C.s3, border: `1px solid ${C.border}`, color: C.textMid }}
              >
                <X size={12} />
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-1.5" style={{ color: C.green }}>
              <CheckCircle2 size={12} />
              <span style={{ fontFamily: F_MONO, fontSize: 11 }}>Approved</span>
            </div>
          )}
        </ArtifactShell>

        <ArtifactShell title="Task List" icon={Layers}>
          <div className="space-y-1.5">
            {content.tasks.map((t, i) => (
              <div key={i} className="flex items-center gap-2">
                {t.done ? <CheckCircle2 size={13} color={C.green} /> : <Circle size={13} color={C.textDim} />}
                <span
                  style={{
                    fontFamily: F_UI,
                    fontSize: 11.5,
                    color: t.done ? C.textMid : C.text,
                    textDecoration: t.done ? "line-through" : "none",
                  }}
                >
                  {t.label}
                </span>
              </div>
            ))}
          </div>
        </ArtifactShell>

        <ArtifactShell title="Diff Cards" icon={GitBranch}>
          <div className="space-y-1.5">
            {content.diffs.map((d) => {
              const status = d.accepted ? "accepted" : diffStatus[d.file];
              if (status === "accepted") {
                return (
                  <div key={d.file} className="flex items-center gap-1.5 fade-in-up" style={{ color: C.green }}>
                    <CheckCircle2 size={12} />
                    <span style={{ fontFamily: F_MONO, fontSize: 10.5 }}>{d.file}</span>
                  </div>
                );
              }
              if (status === "rejected") {
                return (
                  <div key={d.file} className="flex items-center gap-1.5 fade-in-up" style={{ color: C.textDim }}>
                    <X size={12} />
                    <span style={{ fontFamily: F_MONO, fontSize: 10.5, textDecoration: "line-through" }}>{d.file}</span>
                  </div>
                );
              }
              return (
                <div
                  key={d.file}
                  className="rounded-md px-2 py-1.5"
                  style={{ background: C.s3, border: `1px solid ${C.border}` }}
                >
                  <div className="flex items-center justify-between mb-1.5">
                    <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.text }}>{d.file}</span>
                    <span style={{ fontFamily: F_MONO, fontSize: 10 }}>
                      <span style={{ color: C.green }}>+{d.add}</span> <span style={{ color: C.red }}>-{d.del}</span>
                    </span>
                  </div>
                  <div className="flex gap-1">
                    <button
                      onClick={() => accept(d.file)}
                      className="flex-1 rounded py-1"
                      style={{ background: C.greenBg, color: C.green, fontSize: 10, fontFamily: F_UI }}
                    >
                      Accept
                    </button>
                    <button
                      onClick={() => reject(d.file)}
                      className="flex-1 rounded py-1"
                      style={{ background: C.s2, color: C.textDim, fontSize: 10, fontFamily: F_UI }}
                    >
                      Reject
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </ArtifactShell>

        <ArtifactShell title="Verification" icon={ShieldCheck}>
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-1" style={{ color: C.green }}>
              <CheckCircle2 size={12} />
              <span style={{ fontFamily: F_MONO, fontSize: 11 }}>{content.verification.pass} pass</span>
            </div>
            {content.verification.fail > 0 && (
              <div className="flex items-center gap-1" style={{ color: C.red }}>
                <AlertTriangle size={12} />
                <span style={{ fontFamily: F_MONO, fontSize: 11 }}>{content.verification.fail} fail</span>
              </div>
            )}
            {content.verification.running && (
              <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>· re-running…</span>
            )}
          </div>
        </ArtifactShell>

        {content.walkthrough && (
          <ArtifactShell title="Walkthrough" icon={FileCode}>
            <p style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid, lineHeight: 1.6 }}>{content.walkthrough}</p>
          </ArtifactShell>
        )}
      </div>
    </div>
  );
}

/* ---------- command palette ---------- */
function CommandPalette({ onClose }) {
  const items = [
    { icon: FileCode, label: "middleware.rs", sub: "Open file" },
    { icon: Sparkles, label: "Ask Leo: “explain this stack trace”", sub: "Agent View" },
    { icon: Terminal, label: "Run tests", sub: "Task" },
    { icon: Palette, label: "Open Design View", sub: "Mode" },
  ];
  return (
    <div
      className="fixed inset-0 flex items-start justify-center pt-32 z-50"
      style={{ background: "rgba(0,0,0,0.5)", backdropFilter: "blur(2px)" }}
      onClick={onClose}
    >
      <div
        className="w-full max-w-lg rounded-xl overflow-hidden fade-in-up"
        style={{ background: C.s1, border: `1px solid ${C.borderLt}` }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-3" style={{ borderBottom: `1px solid ${C.border}` }}>
          <Search size={14} color={C.textDim} />
          <input
            autoFocus
            placeholder="Search files, commands, or ask Leo…"
            className="flex-1 bg-transparent outline-none"
            style={{ fontFamily: F_UI, fontSize: 13, color: C.text }}
          />
        </div>
        <div className="py-1.5">
          {items.map((it, i) => {
            const Icon = it.icon;
            return (
              <div key={i} className="flex items-center gap-2.5 px-4 py-2 cursor-pointer hover:bg-white/5">
                <Icon size={13} color={C.textDim} />
                <span style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, flex: 1 }}>{it.label}</span>
                <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>{it.sub}</span>
                <ChevronRight size={12} color={C.textDim} />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

/* ---------- studio placeholder views (§22-26) ---------- */
function StudioStub({ title, desc, icon }) {
  const Icon = icon;
  return (
    <div className="flex-1 flex items-center justify-center h-full">
      <div className="text-center max-w-sm">
        <div
          className="mx-auto mb-3 rounded-lg flex items-center justify-center"
          style={{ width: 40, height: 40, background: C.accentBg }}
        >
          <Icon size={18} color={C.accent} />
        </div>
        <div style={{ fontFamily: F_UI, fontWeight: 600, fontSize: 14, color: C.text }}>{title}</div>
        <p style={{ fontFamily: F_UI, fontSize: 12, color: C.textDim, marginTop: 6, lineHeight: 1.6 }}>{desc}</p>
      </div>
    </div>
  );
}
const TestView = () => (
  <StudioStub
    icon={Layers}
    title="Test Studio"
    desc="Universal test explorer, coverage heatmap, visual regression, and the flaky-test dashboard — §24."
  />
);
const OpsView = () => (
  <StudioStub
    icon={GitBranch}
    title="Ops View"
    desc="CI/CD pipeline editor, infra-as-code diffs, Kubernetes panel, one-click cloud deploy tasks (§23), and the read-only Ops Cockpit companion for secondary-screen monitoring (§54)."
  />
);
const DataView = () => (
  <StudioStub
    icon={HardDrive}
    title="Data / ML View"
    desc="Notebook mode, experiment tracking, dataset explorer, the model registry (§25), and Neural Link's local workspace-analysis reports (§53)."
  />
);
const ManageView = () => (
  <StudioStub
    icon={Clock}
    title="Manage View"
    desc="Kanban synced to branches/PRs, two-way issue tracker sync, standup digest generation — §26."
  />
);

/* ---------- settings panel (§42) ---------- */
const SETTINGS_CATEGORIES = [
  { id: "general", label: "General", icon: Sparkles, ref: "§18, §37.9" },
  { id: "appearance", label: "Appearance", icon: Palette, ref: "§8.2-8.4" },
  { id: "models", label: "Leo & Models", icon: Cloud, ref: "§3, §41, §44" },
  { id: "apikeys", label: "API Keys & Credentials", icon: KeyRound, ref: "§58" },
  { id: "agent", label: "Agent Behavior", icon: ShieldCheck, ref: "§4.1, §45" },
  { id: "memory", label: "Memory", icon: Layers, ref: "§4.3" },
  { id: "privacy", label: "Privacy & Security", icon: ShieldCheck, ref: "§9, §27, §50.2" },
  { id: "languages", label: "Languages & Toolchains", icon: Code2, ref: "§20.1" },
  { id: "debugging", label: "Debugging", icon: AlertTriangle, ref: "§32" },
  { id: "adb", label: "ADB & Devices", icon: Terminal, ref: "§33" },
  { id: "design", label: "Design View", icon: PenTool, ref: "§34, §38" },
  { id: "plugins", label: "Plugins & Extensions", icon: ComponentIcon, ref: "§5, §36.4.9" },
  { id: "skills", label: "Skills", icon: Boxes, ref: "§63" },
  { id: "mcp", label: "MCP Servers", icon: Cable, ref: "§64" },
  { id: "notifications", label: "Notifications", icon: Clock, ref: "§37.7" },
  { id: "keybindings", label: "Keybindings", icon: LayoutPanelLeft, ref: "§19" },
  { id: "accessibility", label: "Accessibility", icon: CheckCircle2, ref: "§16.3" },
  { id: "cli", label: "CLI & Remote", icon: Terminal, ref: "§46" },
  { id: "fleet", label: "External Agent Fleet", icon: Terminal, ref: "§52" },
  { id: "import", label: "Import & Migration", icon: Download, ref: "§70" },
  { id: "iot", label: "IoT & Embedded", icon: Cpu, ref: "§72" },
  { id: "auditor", label: "Security Auditor", icon: Bug, ref: "§73" },
  { id: "decompiler", label: "Decompiler", icon: Binary, ref: "§74" },
  { id: "advanced", label: "Advanced", icon: Folder, ref: "catch-all" },
];

// §50.3 declutter pass: a boxed pill on every single settings row added up
// to a lot of simultaneous chrome across a long list — dropping the
// background/border and keeping only the color-coded text still carries the
// same information at a glance, with far less visual noise.
function LayerTag({ layer }) {
  const colors = { System: C.textDim, Global: C.teal, Project: C.accent, Session: C.amber };
  return <span style={{ color: colors[layer], fontFamily: F_MONO, fontSize: 9.5 }}>{layer}</span>;
}
function SettingRow({ label, layer, children }) {
  return (
    <div className="flex items-center justify-between py-2.5" style={{ borderBottom: `1px solid ${C.border}` }}>
      <div className="flex items-center gap-2">
        <span style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text }}>{label}</span>
        {layer && <LayerTag layer={layer} />}
      </div>
      {children}
    </div>
  );
}
function Toggle({ on, onClick }) {
  return (
    <button
      onClick={onClick}
      className="rounded-full transition-colors"
      style={{ width: 34, height: 19, background: on ? C.accent : C.s3, position: "relative" }}
    >
      <span
        className="rounded-full absolute transition-all"
        style={{ width: 14, height: 14, background: "#fff", top: 2.5, left: on ? 17 : 2.5 }}
      />
    </button>
  );
}

// §58: one findable home for credential management, not scattered across
// Leo & Models / Privacy & Security / wherever a feature happened to be
// introduced. Everything here is OS-keychain-backed, never plaintext (§27).
const CREDENTIALS = [
  {
    id: "anthropic",
    label: "Anthropic API key",
    masked: "sk-ant-••••••7f2a",
    lastUsed: "used by Leo · Claude Sonnet, 2m ago",
  },
  {
    id: "github",
    label: "GitHub",
    masked: "OAuth · repo, read:org scopes",
    lastUsed: "used by Source Control §56, 5m ago",
    isOAuth: true,
  },
  {
    id: "huggingface",
    label: "Hugging Face token",
    masked: "hf_••••••q9Lm",
    lastUsed: "used by Model Manager §41, 1h ago",
  },
  {
    id: "litellm-azure",
    label: "Azure OpenAI (via LiteLLM)",
    masked: "••••••9c3d",
    lastUsed: "used by GPT-4.1 fallback chain, 3h ago",
  },
];

function ApiKeysSettings() {
  const [revealed, setRevealed] = useState(new Set());
  const [removed, setRemoved] = useState(new Set());

  function toggleReveal(id) {
    setRevealed((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }
  function remove(id) {
    setRemoved((prev) => new Set(prev).add(id));
  }

  const visible = CREDENTIALS.filter((c) => !removed.has(c.id));

  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 12, lineHeight: 1.6 }}>
        Stored via the OS keychain, never in plaintext config, never sent to Leo's context window (§27, §58.2).
        Revealing a value requires re-authentication where the OS supports it.
      </p>
      <div className="space-y-1.5">
        {visible.map((c) => {
          const isRevealed = revealed.has(c.id);
          return (
            <div
              key={c.id}
              className="rounded-md px-2.5 py-2"
              style={{ background: C.s3, border: `1px solid ${C.border}` }}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <KeyRound size={12} color={C.text} />
                  <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.text }}>{c.label}</span>
                </div>
                <div className="flex items-center gap-2">
                  {!c.isOAuth && (
                    <button onClick={() => toggleReveal(c.id)} title="Reveal (requires re-authentication)">
                      {isRevealed ? <EyeOff size={13} color={C.textDim} /> : <Eye size={13} color={C.textDim} />}
                    </button>
                  )}
                  <button onClick={() => remove(c.id)} title={c.isOAuth ? "Revoke" : "Remove"}>
                    <Trash2 size={13} color={C.textDim} />
                  </button>
                </div>
              </div>
              <div className="flex items-center justify-between mt-1.5">
                <span style={{ fontFamily: F_MONO, fontSize: 11, color: isRevealed ? C.teal : C.textMid }}>
                  {isRevealed ? c.masked.replace(/•+/, "sk-ant-api03-9fJ2kQ") : c.masked}
                </span>
                <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>{c.lastUsed}</span>
              </div>
            </div>
          );
        })}
      </div>
      <div
        className="mt-3 rounded-md px-2.5 py-2 flex items-center gap-2 cursor-pointer"
        style={{ border: `1px dashed ${C.borderLt}` }}
      >
        <Plus size={12} color={C.textDim} />
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>
          Add API key or connect a service — tests the connection before saving (§58.2)
        </span>
      </div>
    </div>
  );
}

function AgentBehaviorSettings({ devMode, onRequestDevMode }) {
  const [autonomy, setAutonomy] = useState("plan-approve");
  const [quarantine, setQuarantine] = useState(true);
  const levels = [
    { id: "manual", label: "Manual" },
    { id: "plan-approve", label: "Plan-Approve" },
    { id: "autonomous", label: "Autonomous" },
    { id: "vibe", label: "Vibe" },
  ];
  return (
    <div>
      <div className="mb-4">
        <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Autonomy level</div>
        <div className="flex gap-1.5 flex-wrap">
          {levels.map((l) => {
            const active = autonomy === l.id;
            const isVibe = l.id === "vibe";
            return (
              <button
                key={l.id}
                onClick={() => setAutonomy(l.id)}
                className="rounded-md px-3 py-1.5"
                style={{
                  fontFamily: F_UI,
                  fontSize: 11.5,
                  background: active ? (isVibe ? C.tealBg : C.accentBg) : C.s3,
                  color: active ? (isVibe ? C.teal : C.accent) : C.textMid,
                  border: `1px solid ${active ? (isVibe ? "rgba(61,156,147,0.4)" : C.accentBorder) : C.border}`,
                }}
              >
                {l.label}
              </button>
            );
          })}
        </div>
        {autonomy === "vibe" && (
          <p
            className="fade-in-up"
            style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginTop: 8, lineHeight: 1.6 }}
          >
            No per-diff approval — checkpointing, path-jailing, and quarantine still apply underneath, unconditionally
            (§45.2).
          </p>
        )}
      </div>
      <SettingRow label="Untrusted-repo quarantine" layer="Global">
        <Toggle on={quarantine} onClick={() => setQuarantine(!quarantine)} />
      </SettingRow>
      <SettingRow label="Plan-scope enforcement" layer="Project">
        <Toggle on={true} onClick={() => {}} />
      </SettingRow>
      <SettingRow label="Sub-agent concurrency limit" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.textMid }}>3</span>
      </SettingRow>

      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginTop: 20, marginBottom: 4 }}>
        Developer Mode · §60
      </div>
      <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        Removes the path jail and standing approval prompts for this workspace only (§60.2.1). Does{" "}
        <b style={{ color: C.textMid }}>not</b> disable destructive-action approval or plugin sandboxing — those stay
        hard invariants at every revision (§60.1).
      </p>
      <SettingRow label="Developer Mode (this workspace)" layer="Project">
        <Toggle on={devMode} onClick={onRequestDevMode} />
      </SettingRow>
      {devMode && (
        <div
          className="rounded-md px-2.5 py-2 mt-2 fade-in-up"
          style={{ background: C.redBg, border: "1px solid rgba(178,69,59,0.4)" }}
        >
          <div className="flex items-center gap-1.5 mb-1">
            <ShieldAlert size={12} color={C.red} />
            <span style={{ fontFamily: F_UI, fontSize: 11, fontWeight: 600, color: C.red }}>
              Active — no path jail for this workspace
            </span>
          </div>
          <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textMid, lineHeight: 1.5 }}>
            No path allowlist, standing env/secrets passthrough, relaxed fetch gating, and no approval for
            reads/edits/non-destructive commands. Destructive actions still require one approval. Every relaxed action
            is tagged in the audit log (§60.3). Not sticky — opening a different project won't carry this over.
          </p>
        </div>
      )}
    </div>
  );
}

// §3.3's curated manifest — role tags, not a raw library dump.
const TAG_META = {
  "coding-fast": { color: C.teal, bg: C.tealBg },
  "coding-strong": { color: C.accent, bg: C.accentBg },
  general: { color: C.textMid, bg: C.s3 },
  embedding: { color: C.amber, bg: C.amberBg },
};
const CURATED_MODELS = [
  { id: "qwen-coder-7b", label: "Qwen2.5-Coder 7B", tag: "coding-fast", size: "4.4 GB", quant: "Q4_K_M" },
  { id: "qwen-coder-32b", label: "Qwen2.5-Coder 32B", tag: "coding-strong", size: "19.8 GB", quant: "Q4_K_M" },
  {
    id: "deepseek-coder-v2-16b",
    label: "DeepSeek-Coder-V2 16B",
    tag: "coding-strong",
    size: "9.9 GB",
    quant: "Q4_K_M",
  },
  { id: "llama-3-1-8b", label: "Llama 3.1 8B", tag: "general", size: "4.9 GB", quant: "Q4_K_M" },
  { id: "nomic-embed", label: "Nomic Embed Text", tag: "embedding", size: "274 MB", quant: "F16" },
];
// §41.2's Hugging Face tab — result cards with quant variants, a
// hardware-fit badge, downloads, and license, not a bare filename list.
const HF_RESULTS = [
  {
    id: "hf-qwen-coder-32b-bartowski",
    repo: "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF",
    params: "32B",
    downloads: "812K",
    license: "apache-2.0",
    updated: "3 weeks ago",
    variants: [
      { tag: "Q4_K_M", size: "19.8 GB", note: "quality/size sweet spot", recommended: true },
      { tag: "Q5_K_M", size: "23.3 GB", note: "closer to full fidelity" },
      { tag: "Q8_0", size: "34.8 GB", note: "max fidelity, ~2x footprint" },
    ],
  },
  {
    id: "hf-deepseek-r1-distill-qwen-14b",
    repo: "unsloth/DeepSeek-R1-Distill-Qwen-14B-GGUF",
    params: "14B",
    downloads: "1.2M",
    license: "mit",
    updated: "2 months ago",
    variants: [
      { tag: "Q4_K_M", size: "9.0 GB", note: "quality/size sweet spot", recommended: true },
      { tag: "Q2_K", size: "5.8 GB", note: "noticeably lossy" },
    ],
  },
];

// §57.3: two local runtimes, one manager — the same curated/HF model can be
// pulled into either, and installed rows show which one actually has it.
const RUNTIME_META = {
  ollama: { label: "Ollama" },
  lmstudio: { label: "LM Studio" },
};

// Which local runtime a pull should land in — §57.3, nothing in the routing
// engine special-cases which one is active, so this is just a destination
// picker, not a capability difference.
function RuntimeToggle({ value, onChange }) {
  return (
    <div
      className="flex items-center gap-0.5 rounded p-0.5"
      style={{ background: C.s2, border: `1px solid ${C.border}` }}
    >
      {Object.entries(RUNTIME_META).map(([id, meta]) => (
        <button
          key={id}
          onClick={() => onChange(id)}
          className="rounded px-1.5 py-0.5"
          style={{
            fontFamily: F_MONO,
            fontSize: 9.5,
            background: value === id ? C.s3 : "transparent",
            color: value === id ? C.text : C.textDim,
          }}
        >
          {meta.label}
        </button>
      ))}
    </div>
  );
}

// §3.3 + §41: a real Installed / Curated / Hugging Face model manager, not a
// single flat list — pull is a streamed Task with real progress, delete
// frees disk, one pipeline serves both entry points (§41.3).
function LocalModelManager() {
  const [tab, setTab] = useState("installed");
  const [installed, setInstalled] = useState({ "qwen-coder-7b": "ollama" }); // id -> runtime
  const [pulls, setPulls] = useState({}); // id -> 0-100
  const [variantPick, setVariantPick] = useState({}); // hf id -> variant tag
  const [runtimePick, setRuntimePick] = useState({}); // model id -> "ollama" | "lmstudio"

  function startPull(id) {
    if (pulls[id] !== undefined || installed[id]) return;
    const runtime = runtimePick[id] || "ollama";
    setPulls((p) => ({ ...p, [id]: 0 }));
    const timer = setInterval(() => {
      setPulls((p) => {
        const cur = p[id];
        if (cur === undefined) {
          clearInterval(timer);
          return p;
        }
        const next = cur + 100 / 7;
        if (next >= 100) {
          clearInterval(timer);
          setInstalled((inst) => ({ ...inst, [id]: runtime }));
          const { [id]: _drop, ...rest } = p;
          return rest;
        }
        return { ...p, [id]: next };
      });
    }, 260);
  }
  function removeModel(id) {
    setInstalled((inst) => {
      const { [id]: _drop, ...rest } = inst;
      return rest;
    });
  }
  function pickRuntime(id, runtime) {
    setRuntimePick((p) => ({ ...p, [id]: runtime }));
  }

  const installedCurated = CURATED_MODELS.filter((m) => installed[m.id]);

  return (
    <div>
      <div
        className="flex items-center gap-0.5 rounded-lg p-0.5 mb-3"
        style={{ background: C.s2, border: `1px solid ${C.border}`, width: "fit-content" }}
      >
        {[
          ["installed", "Installed"],
          ["curated", "Curated"],
          ["huggingface", "Hugging Face"],
        ].map(([id, label]) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className="rounded-md px-2.5 py-1"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: tab === id ? C.accentBg : "transparent",
              color: tab === id ? C.accent : C.textMid,
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "installed" && (
        <div className="space-y-1.5">
          {installedCurated.length === 0 && (
            <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim }}>
              No local models installed yet — pull one from Curated or Hugging Face.
            </p>
          )}
          {installedCurated.map((m) => (
            <div
              key={m.id}
              className="flex items-center justify-between rounded-md px-2.5 py-2"
              style={{ background: C.s3, border: `1px solid ${C.border}` }}
            >
              <div className="flex items-center gap-2">
                <HardDrive size={12} color={C.text} />
                <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>{m.label}</span>
                <span
                  className="rounded px-1 py-0.5"
                  style={{
                    background: (TAG_META[m.tag] || {}).bg,
                    color: (TAG_META[m.tag] || {}).color,
                    fontSize: 9,
                    fontFamily: F_MONO,
                  }}
                >
                  {m.tag}
                </span>
                <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>
                  {m.size} · {m.quant}
                </span>
                <span
                  className="rounded px-1 py-0.5"
                  style={{ background: C.s2, color: C.teal, fontSize: 9, fontFamily: F_MONO }}
                >
                  {(RUNTIME_META[installed[m.id]] || {}).label || installed[m.id]}
                </span>
              </div>
              <button onClick={() => removeModel(m.id)} title="Delete — frees disk (§41.5)">
                <Trash2 size={13} color={C.textDim} />
              </button>
            </div>
          ))}
        </div>
      )}

      {tab === "curated" && (
        <div className="space-y-1.5">
          {CURATED_MODELS.map((m) => {
            const isInstalled = !!installed[m.id];
            const pct = pulls[m.id];
            const runtime = runtimePick[m.id] || "ollama";
            return (
              <div
                key={m.id}
                className="rounded-md px-2.5 py-2"
                style={{ background: C.s3, border: `1px solid ${C.border}` }}
              >
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>{m.label}</span>
                    <span
                      className="rounded px-1 py-0.5"
                      style={{
                        background: (TAG_META[m.tag] || {}).bg,
                        color: (TAG_META[m.tag] || {}).color,
                        fontSize: 9,
                        fontFamily: F_MONO,
                      }}
                    >
                      {m.tag}
                    </span>
                    <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>
                      {m.size} · {m.quant}
                    </span>
                  </div>
                  {isInstalled ? (
                    <span
                      className="flex items-center gap-1"
                      style={{ color: C.green, fontFamily: F_MONO, fontSize: 10.5 }}
                    >
                      <CheckCircle2 size={12} /> Installed · {(RUNTIME_META[installed[m.id]] || {}).label}
                    </span>
                  ) : pct !== undefined ? (
                    <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.teal }}>
                      {Math.round(pct)}% via {(RUNTIME_META[runtime] || {}).label}
                    </span>
                  ) : (
                    <div className="flex items-center gap-1.5">
                      <RuntimeToggle value={runtime} onChange={(r) => pickRuntime(m.id, r)} />
                      <button
                        onClick={() => startPull(m.id)}
                        className="flex items-center gap-1 rounded px-2 py-1"
                        style={{ background: C.accentBg, color: C.accent, fontSize: 10.5, fontFamily: F_UI }}
                      >
                        <Download size={11} /> Pull
                      </button>
                    </div>
                  )}
                </div>
                {pct !== undefined && (
                  <div className="rounded-full mt-2 overflow-hidden fade-in-up" style={{ height: 4, background: C.s2 }}>
                    <div
                      style={{ height: "100%", width: `${pct}%`, background: C.teal, transition: "width 240ms linear" }}
                    />
                  </div>
                )}
              </div>
            );
          })}
          <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, marginTop: 4, lineHeight: 1.6 }}>
            Curated set stays small on purpose and grows via config updates (§3.3) — for the long tail, use Hugging
            Face. A model already installed in one runtime is offered in the other rather than hidden (§57.3).
          </p>
        </div>
      )}

      {tab === "huggingface" && (
        <div>
          <div
            className="flex items-center gap-2 rounded-md px-2.5 py-1.5 mb-3"
            style={{ background: C.s2, border: `1px solid ${C.border}` }}
          >
            <Search size={12} color={C.textDim} />
            <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>
              task:text-generation library:gguf sort:trending
            </span>
          </div>
          <div className="space-y-2">
            {HF_RESULTS.map((r) => {
              const isInstalled = !!installed[r.id];
              const pct = pulls[r.id];
              const runtime = runtimePick[r.id] || "ollama";
              const picked = variantPick[r.id] || r.variants.find((v) => v.recommended)?.tag || r.variants[0].tag;
              return (
                <div
                  key={r.id}
                  className="rounded-md px-2.5 py-2"
                  style={{ background: C.s3, border: `1px solid ${C.border}` }}
                >
                  <div className="flex items-center justify-between mb-1.5">
                    <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>{r.repo}</span>
                    <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>
                      {r.params} · {r.downloads} downloads · {r.license}
                    </span>
                  </div>
                  <div className="flex gap-1 mb-2 flex-wrap">
                    {r.variants.map((v) => (
                      <button
                        key={v.tag}
                        onClick={() => setVariantPick((p) => ({ ...p, [r.id]: v.tag }))}
                        className="rounded px-2 py-1 flex items-center gap-1"
                        style={{
                          fontFamily: F_MONO,
                          fontSize: 10,
                          background: picked === v.tag ? C.accentBg : C.s2,
                          color: picked === v.tag ? C.accent : C.textMid,
                          border: `1px solid ${picked === v.tag ? C.accentBorder : C.border}`,
                        }}
                        title={v.note}
                      >
                        {v.tag} <span style={{ color: C.textDim }}>{v.size}</span>
                        {v.recommended && (
                          <span
                            className="rounded px-1"
                            style={{ background: C.greenBg, color: C.green, fontSize: 8.5 }}
                          >
                            fits your hardware
                          </span>
                        )}
                      </button>
                    ))}
                  </div>
                  <div className="flex items-center justify-between">
                    <span style={{ fontFamily: F_UI, fontSize: 10, color: C.textDim }}>
                      {r.variants.find((v) => v.tag === picked)?.note} · updated {r.updated}
                    </span>
                    {isInstalled ? (
                      <span
                        className="flex items-center gap-1"
                        style={{ color: C.green, fontFamily: F_MONO, fontSize: 10.5 }}
                      >
                        <CheckCircle2 size={12} /> Installed · {(RUNTIME_META[installed[r.id]] || {}).label}
                      </span>
                    ) : pct !== undefined ? (
                      <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.teal }}>
                        {Math.round(pct)}% via {(RUNTIME_META[runtime] || {}).label}
                      </span>
                    ) : (
                      <div className="flex items-center gap-1.5">
                        <RuntimeToggle value={runtime} onChange={(rt) => pickRuntime(r.id, rt)} />
                        <button
                          onClick={() => startPull(r.id)}
                          className="flex items-center gap-1 rounded px-2 py-1"
                          style={{ background: C.accentBg, color: C.accent, fontSize: 10.5, fontFamily: F_UI }}
                        >
                          <Download size={11} /> Pull {picked}
                        </button>
                      </div>
                    )}
                  </div>
                  {pct !== undefined && (
                    <div
                      className="rounded-full mt-2 overflow-hidden fade-in-up"
                      style={{ height: 4, background: C.s2 }}
                    >
                      <div
                        style={{
                          height: "100%",
                          width: `${pct}%`,
                          background: C.teal,
                          transition: "width 240ms linear",
                        }}
                      />
                    </div>
                  )}
                </div>
              );
            })}
          </div>
          <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, marginTop: 8, lineHeight: 1.6 }}>
            Pre-flight disk check runs before any pull begins; safetensors-only repos offer a "Convert &amp; Import"
            flow instead (Downloading → Converting → Quantizing → Registering), never silent (§41.4–§41.5).
          </p>
        </div>
      )}
    </div>
  );
}

function ModelsSettings({ provider, setProvider }) {
  const providers = [
    { id: "claude", label: "Claude Sonnet", type: "Cloud", icon: Cloud, connected: true },
    { id: "ollama", label: "Qwen2.5-Coder (Ollama)", type: "Local", icon: HardDrive, connected: true },
    { id: "lmstudio", label: "LM Studio (localhost:1234)", type: "Local", icon: HardDrive, connected: true },
    { id: "litellm-gpt", label: "GPT-4.1 via LiteLLM", type: "Cloud", icon: Cloud, connected: true },
    { id: "litellm-bedrock", label: "Bedrock Claude 3.5 via LiteLLM", type: "Cloud", icon: Cloud, connected: false },
  ];
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 2 }}>Routing mode</div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10 }}>
        Hybrid — fast completions local, agentic planning cloud (§3.5)
      </p>
      <div
        style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginTop: 16, marginBottom: 8 }}
      >
        CONNECTED PROVIDERS
      </div>
      <div className="space-y-1.5">
        {providers.map((p) => {
          const Icon = p.icon;
          return (
            <div
              key={p.id}
              className="flex items-center justify-between rounded-md px-2.5 py-2"
              style={{ background: C.s3, border: `1px solid ${C.border}` }}
            >
              <div className="flex items-center gap-2">
                <Icon size={12} color={p.connected ? C.text : C.textDim} />
                <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: p.connected ? C.text : C.textDim }}>
                  {p.label}
                </span>
                <span
                  className="rounded px-1 py-0.5"
                  style={{ background: C.s2, fontSize: 9, color: C.textDim, fontFamily: F_MONO }}
                >
                  {p.type}
                </span>
              </div>
              {p.connected ? (
                <CheckCircle2 size={13} color={C.green} />
              ) : (
                <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.accent, cursor: "pointer" }}>Connect</span>
              )}
            </div>
          );
        })}
      </div>
      <div
        style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginTop: 20, marginBottom: 8 }}
      >
        LOCAL MODEL MANAGER · §3.3, §41
      </div>
      <LocalModelManager />
    </div>
  );
}

function PrivacySettings() {
  const [fetchGate, setFetchGate] = useState(true);
  const [gitignoreScope, setGitignoreScope] = useState(true);
  return (
    <div>
      <SettingRow label="Path-jailing" layer="System">
        <span className="flex items-center gap-1" style={{ color: C.green, fontFamily: F_MONO, fontSize: 11 }}>
          <ShieldCheck size={11} /> Always on
        </span>
      </SettingRow>
      <SettingRow label="External content fetch gating" layer="Global">
        <Toggle on={fetchGate} onClick={() => setFetchGate(!fetchGate)} />
      </SettingRow>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginTop: 6, marginBottom: 10, lineHeight: 1.6 }}>
        Rendering an external image/embed found in repo content or model output requires the same approval as a network
        tool call (§50.2).
      </p>
      <SettingRow label="Exclude .gitignore-scoped content from context" layer="Project">
        <Toggle on={gitignoreScope} onClick={() => setGitignoreScope(!gitignoreScope)} />
      </SettingRow>
      <SettingRow label="Secrets vault backend" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.textMid }}>OS Keychain</span>
      </SettingRow>
    </div>
  );
}

function GeneralSettings() {
  const [channel, setChannel] = useState("stable");
  const [telemetry, setTelemetry] = useState(false);
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Update channel</div>
      <div className="flex gap-1.5 mb-4">
        {["stable", "beta", "nightly"].map((c) => (
          <button
            key={c}
            onClick={() => setChannel(c)}
            className="rounded-md px-3 py-1.5 capitalize"
            style={{
              fontFamily: F_UI,
              fontSize: 11.5,
              background: channel === c ? C.accentBg : C.s3,
              color: channel === c ? C.accent : C.textMid,
              border: `1px solid ${channel === c ? C.accentBorder : C.border}`,
            }}
          >
            {c}
          </button>
        ))}
      </div>
      <SettingRow label="Auto-update" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>
          Pinned to {channel} · never forced (§18)
        </span>
      </SettingRow>
      <SettingRow label="Locale" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.textMid }}>en-US</span>
      </SettingRow>
      <SettingRow label="Send anonymous telemetry" layer="Global">
        <Toggle on={telemetry} onClick={() => setTelemetry(!telemetry)} />
      </SettingRow>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginTop: 6, lineHeight: 1.6 }}>
        Off by default. <span style={{ color: C.accent, cursor: "pointer" }}>See exactly what we'd collect →</span>
      </p>
    </div>
  );
}

function AppearanceSettings() {
  const [theme, setTheme] = useState("dark");
  const [reduceMotion, setReduceMotion] = useState(false);
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Theme</div>
      <div className="flex gap-1.5 mb-4">
        {["dark", "light", "system"].map((t) => (
          <button
            key={t}
            onClick={() => setTheme(t)}
            className="rounded-md px-3 py-1.5 capitalize"
            style={{
              fontFamily: F_UI,
              fontSize: 11.5,
              background: theme === t ? C.accentBg : C.s3,
              color: theme === t ? C.accent : C.textMid,
              border: `1px solid ${theme === t ? C.accentBorder : C.border}`,
            }}
          >
            {t}
          </button>
        ))}
      </div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Accent</div>
      <div className="flex gap-2 mb-4">
        {[C.accent, "#3D9C93", "#4E9E72", "#C99A3D", "#6B7FD7"].map((c) => (
          <div
            key={c}
            className="rounded-full cursor-pointer"
            style={{ width: 22, height: 22, background: c, border: c === C.accent ? `2px solid ${C.text}` : "none" }}
          />
        ))}
      </div>
      <SettingRow label="UI font" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>Space Grotesk</span>
      </SettingRow>
      <SettingRow label="Code font" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>IBM Plex Mono</span>
      </SettingRow>
      <SettingRow label="Reduce motion" layer="Global">
        <Toggle on={reduceMotion} onClick={() => setReduceMotion(!reduceMotion)} />
      </SettingRow>
      {/* §67: the honest answer to VS Code's theme ecosystem, given the
          locked no-VS-Code-fork decision (CLAUDE.md) rules out running its
          theme JSON directly — convert into Spartan's own token model
          instead of claiming raw compatibility that isn't real. */}
      <div
        className="rounded-md px-2.5 py-2 flex items-center gap-2 cursor-pointer mt-1"
        style={{ border: `1px dashed ${C.borderLt}` }}
      >
        <Download size={12} color={C.textDim} />
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>
          Import a VS Code theme (.json) — converted to Spartan's token model, not run as-is (§67)
        </span>
      </div>
      <RendererSettings />
    </div>
  );
}

// §66.3: forcing CPU rendering is offered; forcing GPU when none is
// detected is not — that would just be a slower way to reach the crash
// auto-detection exists to avoid.
function RendererSettings() {
  const [mode, setMode] = useState("auto");
  const gpuDetected = true; // this reference mockup runs in a browser tab; the real app probes wgpu at startup (§66.1)
  const active = mode === "cpu" || (mode === "auto" && !gpuDetected);
  return (
    <div className="mt-2">
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginTop: 12, marginBottom: 4 }}>
        Renderer · §66
      </div>
      <p style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, marginBottom: 8, lineHeight: 1.6 }}>
        Falls back to CPU software rasterization if no usable GPU backend is found at startup — visible, never a silent
        crash.
      </p>
      <div className="flex gap-1.5 mb-2">
        {[
          ["auto", "Auto-detect"],
          ["gpu", "Force GPU"],
          ["cpu", "Force CPU"],
        ].map(([id, label]) => (
          <button
            key={id}
            onClick={() => setMode(id)}
            className="rounded-md px-3 py-1.5"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: mode === id ? C.accentBg : C.s3,
              color: mode === id ? C.accent : C.textMid,
              border: `1px solid ${mode === id ? C.accentBorder : C.border}`,
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <div
        className="flex items-center gap-2 rounded-md px-2.5 py-2"
        style={{
          background: active ? C.amberBg : C.greenBg,
          border: `1px solid ${active ? "rgba(201,154,61,0.35)" : "rgba(78,158,114,0.35)"}`,
        }}
      >
        {active ? <Cpu size={13} color={C.amber} /> : <MonitorPlay size={13} color={C.green} />}
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: active ? C.amber : C.green }}>
          {active ? "Software rendering — GPU unavailable or forced off" : "GPU rendering active (wgpu)"}
        </span>
      </div>
    </div>
  );
}

function MemorySettings() {
  return (
    <div>
      {[
        {
          tier: "Project",
          file: ".spartan/memory/project.md",
          preview: "- Prefer named exports, no default exports\n- Auth flows always go through middleware.rs",
        },
        {
          tier: "Global",
          file: "~/.spartan/memory/global.md",
          preview: "- Prefers Rust over Go for new backend services",
        },
      ].map((m) => (
        <div
          key={m.tier}
          className="rounded-md mb-3 px-3 py-2.5"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <div className="flex items-center justify-between mb-1.5">
            <span style={{ fontFamily: F_UI, fontSize: 12, fontWeight: 600, color: C.text }}>{m.tier} memory</span>
            <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.accent, cursor: "pointer" }}>Open file</span>
          </div>
          <div style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim, marginBottom: 6 }}>{m.file}</div>
          <pre style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid, whiteSpace: "pre-wrap", margin: 0 }}>
            {m.preview}
          </pre>
        </div>
      ))}
      <SettingRow label="Local preference learning" layer="Global">
        <Toggle on={true} onClick={() => {}} />
      </SettingRow>
    </div>
  );
}

function LanguagesSettings() {
  const langs = [
    { name: "Rust", status: "ok", tool: "rust-analyzer 0.4.2201", fmt: "rustfmt" },
    { name: "TypeScript", status: "ok", tool: "typescript-language-server 4.3.1", fmt: "Prettier" },
    { name: "JSON / CSS / Markdown", status: "ok", tool: "tree-sitter only, no LSP needed", fmt: "Prettier" },
    { name: "Python", status: "ok", tool: "pyright 1.1.390", fmt: "black" },
    { name: "Kotlin", status: "warn", tool: "kotlin-language-server — update available", fmt: "ktlint" },
    { name: "Go", status: "ok", tool: "gopls 0.16.2", fmt: "gofmt" },
    // §20.1.2: named explicitly rather than folded into "and more".
    { name: "Dart / Flutter", status: "ok", tool: "Dart Analysis Server", fmt: "dart format" },
    { name: "Julia", status: "ok", tool: "LanguageServer.jl", fmt: "JuliaFormatter.jl" },
    { name: "PowerShell", status: "ok", tool: "PowerShell Editor Services", fmt: "PSScriptAnalyzer" },
    {
      name: "Crystal",
      status: "warn",
      tool: "no mature LSP yet — tree-sitter highlighting only",
      fmt: "crystal tool format",
    },
  ];
  const [formatOnSave, setFormatOnSave] = useState(true);
  return (
    <div className="space-y-1.5">
      {/* §20.1.1: Prettier is the named default for the JS/TS/web
          LanguageProfile entries, the same way rustfmt already is for
          Rust — shown here per-language instead of only in prose. */}
      <SettingRow label="Format on save" layer="Global">
        <Toggle on={formatOnSave} onClick={() => setFormatOnSave(!formatOnSave)} />
      </SettingRow>
      {langs.map((l) => (
        <div
          key={l.name}
          className="flex items-center justify-between gap-3 rounded-md px-2.5 py-2"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <div className="flex items-center gap-2 flex-shrink-0">
            <span
              className="rounded-full flex-shrink-0"
              style={{ width: 6, height: 6, background: l.status === "ok" ? C.green : C.amber }}
            />
            <span style={{ fontFamily: F_UI, fontSize: 12, color: C.text }}>{l.name}</span>
          </div>
          <div className="flex items-center gap-3 min-w-0">
            <span
              className="truncate min-w-0"
              title={l.tool}
              style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}
            >
              {l.tool}
            </span>
            <span
              className="rounded px-1.5 py-0.5 flex-shrink-0"
              style={{ background: C.accentBg, color: C.accent, fontFamily: F_MONO, fontSize: 9.5 }}
              title="Default formatter (§20.1.1)"
            >
              {l.fmt}
            </span>
          </div>
        </div>
      ))}
      <SettingRow label="Android SDK path" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>~/Library/Android/sdk</span>
      </SettingRow>
      {/* §61.1: WSL is offered as a toolchain root the same way a native
          SDK path is — no dedicated settings surface existed for it before
          this pass, only the workspace-rail entry and devices-panel tab. */}
      <SettingRow label="WSL default distro" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>Ubuntu-22.04 (§61.1)</span>
      </SettingRow>
    </div>
  );
}

function DebuggingSettings() {
  return (
    <div>
      <SettingRow label="Default C/C++/Rust adapter" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>LLDB</span>
      </SettingRow>
      <SettingRow label="Default Python adapter" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>debugpy</span>
      </SettingRow>
      <SettingRow label="Default Go adapter" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>Delve</span>
      </SettingRow>
      {/* §32.2/§32.3: named rather than left implicit — a Windows-native
          crash dump needs a different engine than a DWARF-based one, and
          Dart/Flutter isn't JVM-based despite sitting next to Kotlin. */}
      <SettingRow label="Default Dart/Flutter adapter" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>Dart VM Service Protocol</span>
      </SettingRow>
      <SettingRow label="Windows crash dump (.dmp) analysis" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>WinDbg / CDB</span>
      </SettingRow>
      <SettingRow label="Core dump auto-triage by Leo" layer="Global">
        <Toggle on={true} onClick={() => {}} />
      </SettingRow>
      <SettingRow label="Read-only production attach" layer="Global">
        <Toggle on={true} onClick={() => {}} />
      </SettingRow>
    </div>
  );
}

function AdbSettings() {
  const [confirmDestructive, setConfirmDestructive] = useState(true);
  const [wsaEnabled, setWsaEnabled] = useState(true);
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Connected devices</div>
      <div
        className="rounded-md px-2.5 py-2 mb-3 flex items-center justify-between"
        style={{ background: C.s3, border: `1px solid ${C.border}` }}
      >
        <div className="flex items-center gap-2">
          <span className="rounded-full" style={{ width: 6, height: 6, background: C.green }} />
          <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>Pixel 8 Pro · emulator-5554</span>
        </div>
        <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>Android 15</span>
      </div>
      <div
        className="rounded-md px-2.5 py-2 mb-3 flex items-center gap-2 cursor-pointer"
        style={{ border: `1px dashed ${C.borderLt}` }}
      >
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>
          + Pair new device (wireless debugging)
        </span>
      </div>
      <SettingRow label="Confirm destructive ADB actions" layer="Global">
        <Toggle on={confirmDestructive} onClick={() => setConfirmDestructive(!confirmDestructive)} />
      </SettingRow>
      <SettingRow label="Screen mirror backend" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>scrcpy (§35.7)</span>
      </SettingRow>
      {/* §61.2: WSA had a devices-panel tab before this pass but no
          settings surface of its own. */}
      <SettingRow label="Windows Subsystem for Android" layer="Global">
        <Toggle on={wsaEnabled} onClick={() => setWsaEnabled(!wsaEnabled)} />
      </SettingRow>
    </div>
  );
}

// §72.1: PlatformIO is the default toolchain layer wherever a board
// supports it — Spartan wraps it the same way §32 wraps LLDB/Delve,
// rather than building a second device database from scratch.
const IOT_BOARDS = [
  { name: "ESP32-DevKitC", port: "/dev/cu.usbserial-0001", toolchain: "PlatformIO", rtos: "FreeRTOS" },
  { name: "Raspberry Pi Pico", port: "/dev/cu.usbmodem14201", toolchain: "PlatformIO", rtos: "bare-metal" },
];

function IotSettings() {
  const [otaEnabled, setOtaEnabled] = useState(false);
  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        Board detection reuses the same serial-port enumeration the Devices panel already uses for Android (§33) — not a
        second discovery system. Flashing is a Task (§20.2) like any build step.
      </p>
      <div style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
        DETECTED BOARDS
      </div>
      <div className="space-y-1.5 mb-4">
        {IOT_BOARDS.map((b) => (
          <div
            key={b.name}
            className="flex items-center justify-between gap-3 rounded-md px-2.5 py-2"
            style={{ background: C.s3, border: `1px solid ${C.border}` }}
          >
            <div className="flex items-center gap-2 flex-shrink-0">
              <span className="rounded-full flex-shrink-0" style={{ width: 6, height: 6, background: C.green }} />
              <span style={{ fontFamily: F_UI, fontSize: 12, color: C.text }}>{b.name}</span>
            </div>
            <div className="flex items-center gap-3 min-w-0">
              <span
                className="truncate"
                title={b.port}
                style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}
              >
                {b.port}
              </span>
              <span
                className="rounded px-1.5 py-0.5 flex-shrink-0"
                style={{ background: C.accentBg, color: C.accent, fontFamily: F_MONO, fontSize: 9.5 }}
                title={`RTOS: ${b.rtos} (§72.4)`}
              >
                {b.toolchain}
              </span>
            </div>
          </div>
        ))}
      </div>
      <SettingRow label="Serial monitor default baud rate" layer="Project">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>115200</span>
      </SettingRow>
      <SettingRow label="MQTT Inspector broker" layer="Project">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>mqtt://localhost:1883</span>
      </SettingRow>
      <SettingRow label="OTA updates (network-capable, §9 approval applies)" layer="Global">
        <Toggle on={otaEnabled} onClick={() => setOtaEnabled(!otaEnabled)} />
      </SettingRow>
    </div>
  );
}

function DesignViewSettings() {
  return (
    <div>
      <SettingRow label="Brand file" layer="Project">
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>DESIGN.md</span>
      </SettingRow>
      <SettingRow label="Token export target" layer="Project">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>Tailwind config</span>
      </SettingRow>
      <SettingRow label="Open Design MCP" layer="Global">
        <span className="flex items-center gap-1" style={{ color: C.green, fontFamily: F_MONO, fontSize: 11 }}>
          <CheckCircle2 size={11} /> Connected
        </span>
      </SettingRow>
      <SettingRow label="Accessibility audit overlay" layer="Project">
        <Toggle on={true} onClick={() => {}} />
      </SettingRow>
    </div>
  );
}

// §68.2: the conversion report is the whole point — every capability is
// reported individually as converted or rejected, never a silent partial
// success that looks more complete than it is.
const EXTENSION_IMPORT_REPORT = [
  { capability: "3 commands", status: "ok", note: "registered as Spartan commands" },
  { capability: "2 keybindings", status: "ok", note: "imported into Keybindings settings" },
  { capability: "configuration schema", status: "ok", note: "rendered as real settings controls" },
  {
    capability: "language server activation",
    status: "rejected",
    note: "requires the real vscode API — not run (§68.2)",
  },
];

function PluginsSettings() {
  const plugins = [
    { name: "eslint-bridge", cpu: 3, mem: 12 },
    { name: "spartan-theme-nord", cpu: 0, mem: 2 },
  ];
  const [importReportOpen, setImportReportOpen] = useState(false);
  return (
    <div className="space-y-2">
      {plugins.map((p) => (
        <div
          key={p.name}
          className="rounded-md px-2.5 py-2"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <div className="flex items-center justify-between mb-1">
            <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>{p.name}</span>
            <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, cursor: "pointer" }}>Disable</span>
          </div>
          <div style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>
            CPU {p.cpu}% · {p.mem}MB (§36.4.9 budget-enforced)
          </div>
        </div>
      ))}
      <div
        className="rounded-md px-2.5 py-2 flex items-center gap-2 cursor-pointer"
        style={{ border: `1px dashed ${C.borderLt}` }}
      >
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>+ Browse marketplace</span>
      </div>
      <div
        onClick={() => setImportReportOpen((o) => !o)}
        className="rounded-md px-2.5 py-2 flex items-center gap-2 cursor-pointer"
        style={{ border: `1px dashed ${C.borderLt}` }}
      >
        <Download size={12} color={C.textDim} />
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>
          Import Antigravity/VS Code extension (.vsix) — converts static contributions only (§68)
        </span>
      </div>
      {importReportOpen && (
        <div
          className="rounded-md px-2.5 py-2 fade-in-up"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <div style={{ fontFamily: F_UI, fontSize: 10.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
            CONVERSION REPORT — per capability, never a silent partial success
          </div>
          {EXTENSION_IMPORT_REPORT.map((r, i) => (
            <div key={i} className="flex items-center gap-2 py-0.5">
              {r.status === "ok" ? <Check size={11} color={C.green} /> : <X size={11} color={C.red} />}
              <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: r.status === "ok" ? C.text : C.textDim }}>
                {r.capability}
              </span>
              <span style={{ fontFamily: F_UI, fontSize: 10, color: C.textDim }}>— {r.note}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// §63: lightweight markdown+script capability packages, distinct from a
// compiled WASM plugin (§5) — the import flow validates against SKILL.md's
// schema, never a silent no-op on a malformed manifest.
const INSTALLED_SKILLS = [
  {
    id: "migration-style",
    label: "How this team writes DB migrations",
    scope: "Project",
    enabled: true,
    source: ".spartan/skills/",
  },
  {
    id: "commit-convention",
    label: "Our commit message convention",
    scope: "Project",
    enabled: true,
    source: ".spartan/skills/",
  },
  {
    id: "debug-flaky-checkout",
    label: "How to debug the flaky checkout test",
    scope: "Global",
    enabled: false,
    source: "~/.spartan/skills/",
  },
];
const MARKETPLACE_SKILLS = [
  { id: "conventional-commits", label: "Conventional Commits Enforcer", verified: true, downloads: "44K" },
  { id: "rust-error-handling", label: "Idiomatic Rust Error Handling", verified: true, downloads: "12K" },
  { id: "a11y-review", label: "Accessibility Review Checklist", verified: false, downloads: "3K" },
];

function SkillsSettings() {
  const [tab, setTab] = useState("installed");
  const [skills, setSkills] = useState(INSTALLED_SKILLS);

  function toggleSkill(id) {
    setSkills((prev) => prev.map((s) => (s.id === id ? { ...s, enabled: !s.enabled } : s)));
  }

  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        A skill is markdown + optional scripts, read as context — not compiled or capability-sandboxed like a WASM
        plugin (§5). Script components still run through the same sandboxed tool-execution model (§4.5, §63.3).
      </p>
      <div
        className="flex items-center gap-0.5 rounded-lg p-0.5 mb-3"
        style={{ background: C.s2, border: `1px solid ${C.border}`, width: "fit-content" }}
      >
        {[
          ["installed", "Installed"],
          ["import", "Import"],
          ["marketplace", "Marketplace"],
        ].map(([id, label]) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className="rounded-md px-2.5 py-1"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: tab === id ? C.accentBg : "transparent",
              color: tab === id ? C.accent : C.textMid,
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {tab === "installed" && (
        <div className="space-y-1.5">
          {skills.map((s) => (
            <div
              key={s.id}
              className="flex items-center justify-between rounded-md px-2.5 py-2"
              style={{ background: C.s3, border: `1px solid ${C.border}` }}
            >
              <div className="flex items-center gap-2 min-w-0">
                <Boxes size={12} color={s.enabled ? C.text : C.textDim} className="flex-shrink-0" />
                <span
                  className="truncate"
                  style={{ fontFamily: F_UI, fontSize: 11.5, color: s.enabled ? C.text : C.textDim }}
                >
                  {s.label}
                </span>
                <span
                  className="rounded px-1 py-0.5 flex-shrink-0"
                  style={{ background: C.s2, fontSize: 9, color: C.textDim, fontFamily: F_MONO }}
                >
                  {s.scope}
                </span>
              </div>
              <Toggle on={s.enabled} onClick={() => toggleSkill(s.id)} />
            </div>
          ))}
        </div>
      )}

      {tab === "import" && (
        <div className="space-y-2">
          <div
            className="rounded-md px-2.5 py-2 flex items-center gap-2 cursor-pointer"
            style={{ border: `1px dashed ${C.borderLt}` }}
          >
            <Boxes size={12} color={C.textDim} />
            <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>
              Choose a local folder, .zip, or git URL — validated against SKILL.md before activation (§63.2)
            </span>
          </div>
          <SettingRow label="Default import scope" layer="Global">
            <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>Project</span>
          </SettingRow>
        </div>
      )}

      {tab === "marketplace" && (
        <div className="space-y-1.5">
          {MARKETPLACE_SKILLS.map((s) => (
            <div
              key={s.id}
              className="flex items-center justify-between rounded-md px-2.5 py-2"
              style={{ background: C.s3, border: `1px solid ${C.border}` }}
            >
              <div className="flex items-center gap-2">
                <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.text }}>{s.label}</span>
                {s.verified ? (
                  <span
                    className="flex items-center gap-1 rounded px-1 py-0.5"
                    style={{ background: C.greenBg, color: C.green, fontSize: 9, fontFamily: F_MONO }}
                  >
                    <CheckCircle2 size={9} /> verified
                  </span>
                ) : (
                  <span
                    className="rounded px-1 py-0.5"
                    style={{ background: C.amberBg, color: C.amber, fontSize: 9, fontFamily: F_MONO }}
                  >
                    unverified
                  </span>
                )}
                <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>{s.downloads} installs</span>
              </div>
              <button
                className="rounded px-2 py-1"
                style={{ background: C.accentBg, color: C.accent, fontSize: 10.5, fontFamily: F_UI }}
              >
                Install
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// §64: MCP connections are separate from the credentials used with them
// (§58) — this is the connection/capability surface, not the secrets page.
const MCP_SERVERS = [
  { id: "github-mcp", label: "GitHub MCP Server", transport: "stdio", status: "connected", tools: 12 },
  { id: "open-design-mcp", label: "Open Design", transport: "SSE", status: "connected", tools: 6 },
  { id: "linear-mcp", label: "Linear", transport: "streamable HTTP", status: "unreachable", tools: 0 },
];
const STATUS_COLOR = { connected: C.green, unreachable: C.red, "awaiting approval": C.amber };

function McpSettings() {
  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        Adding or editing a server is a security-relevant diff requiring approval (§36.4.6) — this panel is where that
        approval renders, not a second trust decision.
      </p>
      <div style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginBottom: 8 }}>
        CONNECTED SERVERS
      </div>
      <div className="space-y-1.5 mb-3">
        {MCP_SERVERS.map((s) => (
          <div
            key={s.id}
            className="rounded-md px-2.5 py-2"
            style={{ background: C.s3, border: `1px solid ${C.border}` }}
          >
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Cable size={12} color={C.text} />
                <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.text }}>{s.label}</span>
                <span
                  className="rounded px-1 py-0.5"
                  style={{ background: C.s2, fontSize: 9, color: C.textDim, fontFamily: F_MONO }}
                >
                  {s.transport}
                </span>
              </div>
              <span
                className="flex items-center gap-1"
                style={{ color: STATUS_COLOR[s.status], fontFamily: F_MONO, fontSize: 10.5 }}
              >
                <span className="rounded-full" style={{ width: 6, height: 6, background: STATUS_COLOR[s.status] }} />
                {s.status}
              </span>
            </div>
            <div className="flex items-center justify-between mt-1.5">
              <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>
                {s.tools} tools exposed · per-tool allowlist (§64.1)
              </span>
              <span
                className="flex items-center gap-1"
                style={{ fontFamily: F_UI, fontSize: 10, color: C.accent, cursor: "pointer" }}
              >
                <RefreshCw size={10} /> Health check
              </span>
            </div>
          </div>
        ))}
      </div>
      <div
        className="rounded-md px-2.5 py-2 flex items-center gap-2 cursor-pointer"
        style={{ border: `1px dashed ${C.borderLt}` }}
      >
        <Plus size={12} color={C.textDim} />
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim }}>
          Add server — stdio command, or a URL for SSE/HTTP (§64.1)
        </span>
      </div>
    </div>
  );
}

// §70.1: markers actually found by scanning the currently-open workspace —
// this repo really does have a CLAUDE.md and a .cursorrules-shaped intent,
// so "detected" isn't a fabricated example, it's what a scan here would say.
const IMPORT_SOURCES = [
  { id: "claude-code", label: "Claude Code", markers: ["CLAUDE.md"], detected: true },
  { id: "cursor", label: "Cursor", markers: [".cursorrules", ".cursor/mcp.json"], detected: true },
  { id: "windsurf", label: "Windsurf", markers: [".windsurfrules"], detected: false },
  { id: "copilot", label: "GitHub Copilot", markers: [".github/copilot-instructions.md"], detected: false },
  { id: "cline", label: "Cline", markers: [".clinerules"], detected: false },
  { id: "continue", label: "Continue", markers: [".continue/config.json"], detected: false },
  { id: "aider", label: "Aider", markers: [".aider.conf.yml"], detected: false },
];

// §70.4: the same per-item, never-a-silent-partial-success report pattern
// as §68.2's extension importer — every item states what happened to it.
const IMPORT_REPORTS = {
  "claude-code": [
    {
      item: "CLAUDE.md instructions",
      scope: "Project",
      status: "ok",
      note: "imported as a Project-scoped Skill (§63)",
    },
    {
      item: ".claude/settings.json permissions",
      scope: "Global",
      status: "flagged",
      note: "approval posture — needs your explicit confirmation, never auto-applied (§70.3)",
    },
  ],
  cursor: [
    { item: ".cursorrules", scope: "Project", status: "ok", note: "imported as a Project-scoped Skill (§63)" },
    {
      item: ".cursor/mcp.json (2 servers)",
      scope: "Project",
      status: "ok",
      note: "imported as MCP Server entries (§64) — same protocol shape, high fidelity",
    },
    {
      item: "auto-run approval setting",
      scope: "Global",
      status: "flagged",
      note: "not auto-applied — needs your explicit confirmation (§70.3)",
    },
  ],
  _default: [
    { item: "Custom instructions", scope: "Project", status: "ok", note: "imported as a Skill (§63)" },
    {
      item: "Keybinding preset",
      scope: "Global",
      status: "ok",
      note: "mapped onto the closest matching Keybindings preset (§16)",
    },
    {
      item: "Model/provider preference",
      scope: "Global",
      status: "rejected",
      note: "no equivalent provider connected in Leo & Models (§3, §44)",
    },
    {
      item: 'Auto-approval / "YOLO" setting',
      scope: "Global",
      status: "flagged",
      note: "not auto-applied — needs your explicit confirmation (§70.3)",
    },
  ],
};

function ImportSettings() {
  const [openSource, setOpenSource] = useState(null);
  const detected = IMPORT_SOURCES.filter((s) => s.detected);
  const other = IMPORT_SOURCES.filter((s) => !s.detected);

  function ImportRow({ source }) {
    const open = openSource === source.id;
    return (
      <div>
        <div
          onClick={() => setOpenSource(open ? null : source.id)}
          className="rounded-md px-2.5 py-2 flex items-center justify-between cursor-pointer"
          style={{ background: C.s3, border: `1px solid ${C.border}` }}
        >
          <div className="flex items-center gap-2 min-w-0">
            <span style={{ fontFamily: F_UI, fontSize: 12, color: C.text }}>{source.label}</span>
            <span style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim }}>{source.markers.join(", ")}</span>
          </div>
          <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.accent, flexShrink: 0 }}>Import</span>
        </div>
        {open && (
          <div
            className="rounded-md px-2.5 py-2 mt-1 fade-in-up"
            style={{ background: C.s2, border: `1px solid ${C.border}` }}
          >
            {(IMPORT_REPORTS[source.id] || IMPORT_REPORTS._default).map((r, i) => (
              <div key={i} className="flex items-start gap-2 py-0.5">
                {r.status === "ok" ? (
                  <Check size={11} color={C.green} className="flex-shrink-0" style={{ marginTop: 2 }} />
                ) : r.status === "flagged" ? (
                  <AlertTriangle size={11} color={C.amber} className="flex-shrink-0" style={{ marginTop: 2 }} />
                ) : (
                  <X size={11} color={C.red} className="flex-shrink-0" style={{ marginTop: 2 }} />
                )}
                <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.text }}>
                  {r.item} <span style={{ color: C.textDim }}>({r.scope})</span>
                </span>
                <span style={{ fontFamily: F_UI, fontSize: 10, color: C.textDim }}>— {r.note}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 12, lineHeight: 1.6 }}>
        Converts what's real: instructions/rules become Skills (§63), MCP servers carry over almost losslessly (§64),
        themes and keybinding presets map onto Spartan's own. An approval/autonomy posture never imports silently, no
        matter the source (§70.3).
      </p>
      {detected.length > 0 && (
        <>
          <div style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
            DETECTED IN THIS PROJECT
          </div>
          <div className="space-y-1.5 mb-4">
            {detected.map((s) => (
              <ImportRow key={s.id} source={s} />
            ))}
          </div>
        </>
      )}
      <div style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
        OTHER SOURCES
      </div>
      <div className="space-y-1.5">
        {other.map((s) => (
          <ImportRow key={s.id} source={s} />
        ))}
      </div>
    </div>
  );
}

function NotificationsSettings() {
  const cats = ["CI failures", "Review comments", "Mentions", "Scheduled task results"];
  return (
    <div className="space-y-2">
      {cats.map((c) => (
        <SettingRow key={c} label={c}>
          <Toggle on={true} onClick={() => {}} />
        </SettingRow>
      ))}
      <SettingRow label="Quiet hours" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>10 PM – 8 AM</span>
      </SettingRow>
    </div>
  );
}

function KeybindingsSettings() {
  const [preset, setPreset] = useState("spartan");
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Preset</div>
      <div className="flex gap-1.5 flex-wrap">
        {["spartan", "vs code", "jetbrains", "vim", "emacs"].map((p) => (
          <button
            key={p}
            onClick={() => setPreset(p)}
            className="rounded-md px-3 py-1.5 capitalize"
            style={{
              fontFamily: F_UI,
              fontSize: 11,
              background: preset === p ? C.accentBg : C.s3,
              color: preset === p ? C.accent : C.textMid,
              border: `1px solid ${preset === p ? C.accentBorder : C.border}`,
            }}
          >
            {p}
          </button>
        ))}
      </div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginTop: 10, lineHeight: 1.6 }}>
        Every mouse action has a keyboard equivalent, audited as part of the accessibility release gate (§16.3, §37.11).
      </p>
    </div>
  );
}

function AccessibilitySettings() {
  const [highContrast, setHighContrast] = useState(false);
  const [screenReader, setScreenReader] = useState(true);
  return (
    <div>
      <SettingRow label="High-contrast theme" layer="Global">
        <Toggle on={highContrast} onClick={() => setHighContrast(!highContrast)} />
      </SettingRow>
      <SettingRow label="Screen reader announcements" layer="Global">
        <Toggle on={screenReader} onClick={() => setScreenReader(!screenReader)} />
      </SettingRow>
      <SettingRow label="Minimum tap target size" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>44px</span>
      </SettingRow>
      <SettingRow label="Focus outline style" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>High visibility</span>
      </SettingRow>
    </div>
  );
}

const CLI_COMMANDS = [
  { cmd: 'spartan chat "..."', desc: "One-shot or interactive Q&A, no file writes" },
  { cmd: 'spartan run "<task>"', desc: "Full plan→execute→verify agentic task" },
  { cmd: 'spartan vibe "<task>"', desc: "Runs in Vibe Mode (§45)" },
  { cmd: "spartan plan / diff / approve", desc: "Inspect & act on the current session's artifacts headlessly" },
  { cmd: "spartan status", desc: "List active/recent sessions, mirrors the left rail" },
  { cmd: "spartan mcp serve", desc: "Exposes the current project as an MCP endpoint (§38)" },
  { cmd: "... | spartan explain", desc: "Pipe any output in — logs, stack traces, diffs" },
];

function CliSettings() {
  const [mcpServe, setMcpServe] = useState(false);
  const [explainLast, setExplainLast] = useState(true);
  const [jsonDefault, setJsonDefault] = useState(false);
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8 }}>Operating mode</div>
      <SettingRow label="Companion daemon" layer="System">
        <span className="flex items-center gap-1" style={{ color: C.green, fontFamily: F_MONO, fontSize: 11 }}>
          <CheckCircle2 size={11} /> Running · paired
        </span>
      </SettingRow>
      <p
        style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, marginTop: 4, marginBottom: 10, lineHeight: 1.6 }}
      >
        One Leo, two surfaces — a CLI-initiated task shares this session's store, Project Graph, and memory tiers, and
        shows up live in the left rail (§46.2). Falls back to standalone mode automatically on a machine with no paired
        desktop instance reachable.
      </p>
      <SettingRow label="Auth method" layer="System">
        <span className="flex items-center gap-1" style={{ color: C.textMid, fontFamily: F_MONO, fontSize: 11 }}>
          Reusing desktop's OS keychain
        </span>
      </SettingRow>
      <SettingRow label="Standalone/headless config" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>~/.spartan/cli.toml</span>
      </SettingRow>
      <SettingRow label="Non-interactive approval policy" layer="Project">
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>.spartan/ci-policy.toml</span>
      </SettingRow>

      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginTop: 20, marginBottom: 8 }}>
        Scripting & shell integration
      </div>
      <SettingRow label="Default output format is --json" layer="Global">
        <Toggle on={jsonDefault} onClick={() => setJsonDefault(!jsonDefault)} />
      </SettingRow>
      <p
        style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim, marginTop: -4, marginBottom: 6, lineHeight: 1.6 }}
      >
        Exit codes stay mapped to verification results either way (0 = verified pass) — this only changes the default
        rendering for a human terminal (§46.4).
      </p>
      <SettingRow label="Expose project as MCP endpoint (spartan mcp serve)" layer="Project">
        <Toggle on={mcpServe} onClick={() => setMcpServe(!mcpServe)} />
      </SettingRow>
      <SettingRow label="spartan explain-last shell hook" layer="Global">
        <Toggle on={explainLast} onClick={() => setExplainLast(!explainLast)} />
      </SettingRow>
      <SettingRow label="Shell completions installed" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>bash, zsh, fish</span>
      </SettingRow>

      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginTop: 20, marginBottom: 8 }}>
        Command reference · §46.3
      </div>
      <div className="rounded-md overflow-hidden" style={{ border: `1px solid ${C.border}` }}>
        {CLI_COMMANDS.map((c, i) => (
          <div
            key={c.cmd}
            className="flex items-center gap-3 px-2.5 py-1.5"
            style={{ background: i % 2 ? C.s2 : "transparent", borderTop: i ? `1px solid ${C.border}` : "none" }}
          >
            <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.teal, flexShrink: 0, width: 190 }}>
              {c.cmd}
            </span>
            <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim }}>{c.desc}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function FleetSettings() {
  // Full roster from the legacy console (§55) — every engine that product
  // could launch gets a registry row here, not just whichever two or three
  // happen to be running in the demo session content.
  const engines = [
    { id: "claude-code", label: "Claude Code", detected: true, fallback: "codex" },
    { id: "codex", label: "Codex", detected: true, fallback: "gemini-cli" },
    { id: "aider", label: "Aider", detected: true, fallback: "codex" },
    { id: "gemini-cli", label: "Gemini CLI", detected: true, fallback: null },
    { id: "opencode", label: "OpenCode", detected: true, fallback: null },
    { id: "cursor-agent", label: "Cursor Agent", detected: true, fallback: null },
    { id: "cline", label: "Cline", detected: true, fallback: null },
    { id: "qwen-code", label: "Qwen Code", detected: true, fallback: "codex" },
    { id: "windsurf", label: "Windsurf", detected: false, fallback: null },
    { id: "copilot-cli", label: "GitHub Copilot CLI", detected: false, fallback: null },
    { id: "openai-cli", label: "OpenAI CLI", detected: false, fallback: null },
    { id: "amp", label: "Amp", detected: false, fallback: null },
    { id: "goose", label: "Goose", detected: false, fallback: null },
    { id: "crush", label: "Crush", detected: false, fallback: null },
    { id: "continue", label: "Continue", detected: false, fallback: null },
    { id: "openhands", label: "OpenHands", detected: false, fallback: null },
  ];
  return (
    <div>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 2 }}>Registered engines</div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        Third-party CLIs run as supervised sessions in the same rail as Leo, not as ModelProvider backends — Spartan
        can't see their internal tool-call approvals (§52.1, §52.4).
      </p>
      <div className="space-y-1.5">
        {engines.map((e) => (
          <div
            key={e.id}
            className="flex items-center justify-between rounded-md px-2.5 py-2"
            style={{ background: C.s3, border: `1px solid ${C.border}` }}
          >
            <div className="flex items-center gap-2">
              <Terminal size={12} color={e.detected ? (ENGINE_META[e.id] || {}).color || C.text : C.textDim} />
              <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: e.detected ? C.text : C.textDim }}>
                {e.label}
              </span>
              {e.fallback && (
                <span
                  className="rounded px-1 py-0.5"
                  style={{ background: C.s2, fontSize: 9, color: C.textDim, fontFamily: F_MONO }}
                >
                  → falls back to {e.fallback}
                </span>
              )}
            </div>
            {e.detected ? (
              <span className="flex items-center gap-1" style={{ color: C.green, fontFamily: F_MONO, fontSize: 10.5 }}>
                <CheckCircle2 size={11} /> on $PATH
              </span>
            ) : (
              <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim }}>not found</span>
            )}
          </div>
        ))}
      </div>
      <SettingRow label="Registry file" layer="Project">
        <span style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textMid }}>.spartan/fleet-clis.toml</span>
      </SettingRow>
      <SettingRow label="Auto-switch on quota/rate-limit" layer="Global">
        <span style={{ fontFamily: F_UI, fontSize: 10.5, color: C.textDim }}>
          Prompt to switch (never silent) · §52.3
        </span>
      </SettingRow>
      <SettingRow label="Autonomous approval tier" layer="System">
        <span className="flex items-center gap-1" style={{ color: C.amber, fontFamily: F_MONO, fontSize: 11 }}>
          <AlertTriangle size={11} /> Not eligible for Fleet sessions
        </span>
      </SettingRow>
    </div>
  );
}

// §73.4: two confidence levels, never conflated — a static/flagged finding
// and a verified-exploitable one render distinctly, always.
const AUDIT_FINDINGS = [
  {
    id: "sqli",
    title: "SQL injection in /api/search",
    severity: "critical",
    verified: true,
    path: "src/api/search.rs:42",
    detail:
      "?q=' OR '1'='1 returns every row — confirmed against the local dev instance via the Playwright panel (§65), not inferred from source alone.",
  },
  {
    id: "secret",
    title: "Hardcoded API key in config.rs",
    severity: "high",
    verified: true,
    path: "src/config.rs:18",
    detail:
      "Confirmed loaded at runtime via the env-var fallback path, not dead code — live-severity, not a theoretical exposure.",
  },
  {
    id: "cve-1",
    title: "CVE-2024-31337 in dependency `foo 1.2.0`",
    severity: "medium",
    verified: false,
    path: "Cargo.lock",
    detail: "Flagged by the §27 CVE feed — reachability from this project's own call graph not yet verified (§73.3).",
  },
  {
    id: "iot-cred",
    title: "Default MQTT broker credentials in firmware config",
    severity: "high",
    verified: false,
    path: "firmware/mqtt_config.h",
    detail: "Flagged by the IoT-specific check (§72.5) — not yet verified against a live broker.",
  },
];
const SEVERITY_COLOR = { critical: C.red, high: C.amber, medium: C.teal, low: C.textDim };

function AuditorSettings() {
  const [openFinding, setOpenFinding] = useState(null);
  const [confirming, setConfirming] = useState(false);
  const [lastRun, setLastRun] = useState(null);

  function requestRun() {
    if (confirming) {
      setConfirming(false);
      setLastRun("just now");
    } else {
      setConfirming(true);
    }
  }

  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        Static findings (§27) get verified for real exploitability against a locally-running instance of{" "}
        <strong style={{ color: C.textMid }}>this project only</strong> — never a third-party host, never a URL outside
        this project's own configured local/staging environment (§73.2). Every active-verification run needs its own
        explicit approval below, even under Autonomous or Vibe autonomy (§45).
      </p>
      <div
        className="rounded-md px-2.5 py-2 mb-4 flex items-center justify-between gap-3"
        style={{ background: C.s3, border: `1px solid ${confirming ? C.accentBorder : C.border}` }}
      >
        <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid }}>
          {confirming
            ? "Confirm: verify exploitability against this project's local dev instance only?"
            : lastRun
              ? `Last run: ${lastRun}`
              : "Not yet run"}
        </span>
        <button
          onClick={requestRun}
          className="rounded-md px-3 py-1.5 flex-shrink-0"
          style={{
            background: confirming ? C.accent : C.accentBg,
            color: confirming ? "#fff" : C.accent,
            fontFamily: F_UI,
            fontSize: 11,
          }}
        >
          {confirming ? "Confirm & run" : "Run exploit verification"}
        </button>
      </div>
      <div style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
        FINDINGS
      </div>
      <div className="space-y-1.5">
        {AUDIT_FINDINGS.map((f) => {
          const open = openFinding === f.id;
          return (
            <div key={f.id}>
              <div
                onClick={() => setOpenFinding(open ? null : f.id)}
                className="rounded-md px-2.5 py-2 cursor-pointer"
                style={{ background: C.s3, border: `1px solid ${C.border}` }}
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      className="rounded-full flex-shrink-0"
                      style={{ width: 6, height: 6, background: SEVERITY_COLOR[f.severity] }}
                    />
                    <span className="truncate" style={{ fontFamily: F_UI, fontSize: 12, color: C.text }}>
                      {f.title}
                    </span>
                  </div>
                  {f.verified ? (
                    <span
                      className="flex items-center gap-1 flex-shrink-0"
                      style={{ color: C.red, fontFamily: F_MONO, fontSize: 9.5 }}
                    >
                      <Bug size={10} /> verified exploitable
                    </span>
                  ) : (
                    <span
                      className="flex items-center gap-1 flex-shrink-0"
                      style={{ color: C.textDim, fontFamily: F_MONO, fontSize: 9.5 }}
                    >
                      <AlertTriangle size={10} /> flagged, not yet verified
                    </span>
                  )}
                </div>
                <div style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim, marginTop: 3 }}>{f.path}</div>
              </div>
              {open && (
                <div
                  className="rounded-md px-2.5 py-2 mt-1 fade-in-up"
                  style={{ background: C.s2, border: `1px solid ${C.border}` }}
                >
                  <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textMid, lineHeight: 1.6, marginBottom: 8 }}>
                    {f.detail}
                  </p>
                  {f.verified && (
                    <button
                      className="rounded-md px-2.5 py-1"
                      style={{ background: C.accentBg, color: C.accent, fontFamily: F_UI, fontSize: 10.5 }}
                    >
                      Ask Leo to propose a fix
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// §74.2: same registry pattern as LanguageProfile/DebugAdapterProfile/
// IoTBoardProfile — wrap the strongest existing open source engine per
// format rather than building a second decompiler from scratch.
const DECOMPILER_ENGINES = [
  { name: "Ghidra", formats: "ELF, PE, Mach-O — x86/ARM/MIPS/RISC-V", detected: true, primary: true },
  { name: "radare2", formats: "Fast triage — disassembly, strings, CFG", detected: true, primary: false },
  { name: "JADX", formats: "Android APK / DEX (§21)", detected: true, primary: false },
  { name: "CFR / Fernflower", formats: "JVM .class / .jar (§20)", detected: false, primary: false },
  { name: "ILSpy", formats: ".NET / CIL assemblies (§20)", detected: false, primary: false },
];

function DecompilerSettings() {
  const [confirming, setConfirming] = useState(false);
  const [decompiled, setDecompiled] = useState(false);

  function requestDecompile() {
    if (confirming) {
      setConfirming(false);
      setDecompiled(true);
    } else {
      setConfirming(true);
    }
  }

  return (
    <div>
      <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginBottom: 10, lineHeight: 1.6 }}>
        Read-only pseudocode/disassembly, never written back into the binary. A binary that isn't a build artifact of
        this already-trusted project is treated as untrusted content — the same Quarantine posture (§36.4.2) applied
        here rather than a second policy, so nothing extracted from it auto-runs and any embedded URL still needs
        fetch-gating approval (§50.2).
      </p>
      <div style={{ fontFamily: F_UI, fontSize: 11.5, fontWeight: 600, color: C.textDim, marginBottom: 6 }}>
        ENGINES
      </div>
      <div className="space-y-1.5 mb-4">
        {DECOMPILER_ENGINES.map((e) => (
          <div
            key={e.name}
            className="flex items-center justify-between gap-3 rounded-md px-2.5 py-2"
            style={{ background: C.s3, border: `1px solid ${C.border}` }}
          >
            <div className="flex items-center gap-2 min-w-0">
              <span
                className="rounded-full flex-shrink-0"
                style={{ width: 6, height: 6, background: e.detected ? C.green : C.textDim }}
              />
              <span style={{ fontFamily: F_UI, fontSize: 12, color: e.detected ? C.text : C.textDim }}>{e.name}</span>
              {e.primary && (
                <span
                  className="rounded px-1.5 py-0.5 flex-shrink-0"
                  style={{ background: C.accentBg, color: C.accent, fontFamily: F_MONO, fontSize: 9 }}
                >
                  default
                </span>
              )}
            </div>
            <span className="truncate" title={e.formats} style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>
              {e.detected ? e.formats : "not found on $PATH"}
            </span>
          </div>
        ))}
      </div>
      <div
        className="rounded-md px-2.5 py-2 mb-3 flex items-center justify-between gap-3"
        style={{ background: C.s3, border: `1px solid ${confirming ? C.accentBorder : C.border}` }}
      >
        <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid }}>
          {confirming
            ? "Confirm: treat as untrusted content, no auto-run (§74.7)?"
            : decompiled
              ? "libcheckout.so decompiled — read-only view opened"
              : "No binary selected"}
        </span>
        <button
          onClick={requestDecompile}
          className="rounded-md px-3 py-1.5 flex-shrink-0"
          style={{
            background: confirming ? C.accent : C.accentBg,
            color: confirming ? "#fff" : C.accent,
            fontFamily: F_UI,
            fontSize: 11,
          }}
        >
          {confirming ? "Confirm & decompile" : "Decompile a binary…"}
        </button>
      </div>
      {decompiled && (
        <div
          className="rounded-md px-3 py-2.5 fade-in-up"
          style={{ background: "#0d0d10", border: `1px solid ${C.borderLt}` }}
        >
          <div style={{ fontFamily: F_MONO, fontSize: 9.5, color: C.textDim, marginBottom: 4 }}>
            Ghidra · libcheckout.so · read-only
          </div>
          <pre
            style={{
              fontFamily: F_MONO,
              fontSize: 10.5,
              color: C.textMid,
              whiteSpace: "pre-wrap",
              margin: 0,
              lineHeight: 1.6,
            }}
          >
            {`undefined8 validate_discount(char *param_1) {
  undefined8 result;
  if (strcmp(param_1, "STAFF50") == 0) {
    result = 1;  // hardcoded discount code, no server-side check
  } else {
    result = check_remote(param_1);
  }
  return result;
}`}
          </pre>
        </div>
      )}
    </div>
  );
}

function AdvancedSettings() {
  const [neuralLink, setNeuralLink] = useState(false);
  const [opsCockpit, setOpsCockpit] = useState(false);
  return (
    <div>
      {/* §53/§54 previously only surfaced as descriptive text inside the
          Ops/Data workspace stubs — no settings toggle existed for either
          before this pass. */}
      <SettingRow label="Neural Link workspace analysis (§53)" layer="Project">
        <Toggle on={neuralLink} onClick={() => setNeuralLink(!neuralLink)} />
      </SettingRow>
      <SettingRow label="Ops Cockpit companion pairing (§54)" layer="Global">
        <Toggle on={opsCockpit} onClick={() => setOpsCockpit(!opsCockpit)} />
      </SettingRow>
      <div style={{ fontFamily: F_UI, fontSize: 12.5, color: C.text, marginBottom: 8, marginTop: 16 }}>
        Raw config.toml
      </div>
      <pre
        className="rounded-md px-3 py-2.5"
        style={{
          background: C.s3,
          border: `1px solid ${C.border}`,
          fontFamily: F_MONO,
          fontSize: 10.5,
          color: C.textMid,
          overflowX: "auto",
        }}
      >
        {`[routing]
mode = "hybrid"
inline_provider = "ollama"
agentic_provider = "claude"

[agent]
autonomy = "plan-approve"`}
      </pre>
      <SettingRow label="Experimental feature flags" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>0 enabled</span>
      </SettingRow>
      <SettingRow label="Diagnostic log level" layer="Global">
        <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textMid }}>info</span>
      </SettingRow>
    </div>
  );
}

// §60.3: enabling Developer Mode requires the full security-relevant
// change confirmation (§42.1/§42.3), not a casual toggle-and-forget —
// disabling it needs no such ceremony, since narrowing trust back down
// is never the risky direction.
function DevModeConfirmModal({ onConfirm, onCancel }) {
  return (
    <div
      className="fixed inset-0 flex items-center justify-center z-50"
      style={{ background: "rgba(0,0,0,0.55)", backdropFilter: "blur(2px)" }}
      onClick={onCancel}
    >
      <div
        className="w-full max-w-md rounded-xl overflow-hidden fade-in-up"
        style={{ background: C.s1, border: `1px solid ${C.borderLt}` }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex items-center gap-2 px-4 py-3"
          style={{ borderBottom: `1px solid ${C.border}`, background: C.redBg }}
        >
          <ShieldAlert size={15} color={C.red} />
          <span style={{ fontFamily: F_UI, fontSize: 13, fontWeight: 600, color: C.red }}>
            Enable Developer Mode for this workspace?
          </span>
        </div>
        <div className="px-4 py-3">
          <p style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid, lineHeight: 1.6, marginBottom: 10 }}>
            This will, for this workspace only (§60.2, revised §60.2.1):
          </p>
          <ul className="space-y-1.5 mb-3">
            {[
              "Remove the path allowlist entirely — any path your OS user account can reach, not jailed to an expanded set",
              "Make env/secrets passthrough to Leo's terminal tool standing instead of per-session",
              "Relax external content fetch gating (§50.2) from per-fetch approval to a standing allow",
              "Skip approval for reads, edits, and non-destructive shell commands entirely",
            ].map((t) => (
              <li key={t} className="flex items-start gap-1.5">
                <Check size={11} color={C.amber} className="flex-shrink-0 mt-0.5" />
                <span style={{ fontFamily: F_UI, fontSize: 11, color: C.text }}>{t}</span>
              </li>
            ))}
          </ul>
          <p style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, lineHeight: 1.6, marginBottom: 14 }}>
            <b style={{ color: C.textMid }}>Never changes, at any revision:</b> destructive actions (
            <span style={{ fontFamily: F_MONO }}>rm -rf</span>,{" "}
            <span style={{ fontFamily: F_MONO }}>git push --force</span>, migrations,{" "}
            <span style={{ fontFamily: F_MONO }}>sudo</span>) still require one explicit approval, and plugin sandboxing
            is unaffected (§60.1). The first action outside the project directory still surfaces one confirmation before
            being remembered for the session. A persistent indicator stays visible in the top rail the whole time this
            is active.
          </p>
          <div className="flex gap-2">
            <button
              onClick={onConfirm}
              className="flex-1 rounded-md py-2"
              style={{ background: C.red, fontFamily: F_UI, fontSize: 12, color: "#fff" }}
            >
              Enable for this workspace
            </button>
            <button
              onClick={onCancel}
              className="rounded-md px-4"
              style={{
                background: C.s3,
                border: `1px solid ${C.border}`,
                color: C.textMid,
                fontFamily: F_UI,
                fontSize: 12,
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function SettingsView({ onClose, devMode, setDevMode }) {
  const [cat, setCat] = useState("agent");
  const [provider, setProvider] = useState("cloud");
  const [confirmOpen, setConfirmOpen] = useState(false);
  const current = SETTINGS_CATEGORIES.find((c) => c.id === cat);

  function requestDevMode() {
    if (devMode) {
      setDevMode(false); // narrowing back down needs no ceremony
    } else {
      setConfirmOpen(true);
    }
  }

  return (
    <div className="flex flex-1 min-h-0 fade-in-up">
      <div
        className="w-56 flex-shrink-0 overflow-y-auto py-3 px-2"
        style={{ borderRight: `1px solid ${C.border}`, background: C.s1 }}
      >
        <div className="flex items-center justify-between px-2 mb-2">
          <span style={{ fontFamily: F_UI, fontSize: 11, fontWeight: 600, color: C.textDim, letterSpacing: "0.06em" }}>
            SETTINGS
          </span>
          <button onClick={onClose}>
            <X size={13} color={C.textDim} />
          </button>
        </div>
        {SETTINGS_CATEGORIES.map((c) => {
          const Icon = c.icon;
          const active = cat === c.id;
          return (
            <button
              key={c.id}
              onClick={() => setCat(c.id)}
              className="w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left"
              style={{ background: active ? C.s3 : "transparent", color: active ? C.text : C.textMid }}
            >
              <Icon size={12} color={active ? C.accent : C.textDim} />
              <span style={{ fontFamily: F_UI, fontSize: 11.5 }}>{c.label}</span>
            </button>
          );
        })}
      </div>
      <div className="flex-1 overflow-y-auto px-8 py-6" style={{ background: C.bg }}>
        <div className="max-w-lg">
          <div className="flex items-center gap-2 mb-1">
            <span style={{ fontFamily: F_UI, fontSize: 16, fontWeight: 600, color: C.text }}>{current.label}</span>
            <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim }}>{current.ref}</span>
          </div>
          <div className="mt-5">
            {cat === "general" && <GeneralSettings />}
            {cat === "appearance" && <AppearanceSettings />}
            {cat === "models" && <ModelsSettings provider={provider} setProvider={setProvider} />}
            {cat === "apikeys" && <ApiKeysSettings />}
            {cat === "agent" && <AgentBehaviorSettings devMode={devMode} onRequestDevMode={requestDevMode} />}
            {cat === "memory" && <MemorySettings />}
            {cat === "privacy" && <PrivacySettings />}
            {cat === "languages" && <LanguagesSettings />}
            {cat === "debugging" && <DebuggingSettings />}
            {cat === "adb" && <AdbSettings />}
            {cat === "design" && <DesignViewSettings />}
            {cat === "plugins" && <PluginsSettings />}
            {cat === "skills" && <SkillsSettings />}
            {cat === "mcp" && <McpSettings />}
            {cat === "notifications" && <NotificationsSettings />}
            {cat === "keybindings" && <KeybindingsSettings />}
            {cat === "accessibility" && <AccessibilitySettings />}
            {cat === "cli" && <CliSettings />}
            {cat === "fleet" && <FleetSettings />}
            {cat === "import" && <ImportSettings />}
            {cat === "iot" && <IotSettings />}
            {cat === "auditor" && <AuditorSettings />}
            {cat === "decompiler" && <DecompilerSettings />}
            {cat === "advanced" && <AdvancedSettings />}
          </div>
        </div>
      </div>
      {confirmOpen && (
        <DevModeConfirmModal
          onConfirm={() => {
            setDevMode(true);
            setConfirmOpen(false);
          }}
          onCancel={() => setConfirmOpen(false)}
        />
      )}
    </div>
  );
}

/* ---------- root ---------- */
export default function SpartanPrototype() {
  const [mode, setMode] = useState("agent");
  const [fading, setFading] = useState(false);
  const [activeId, setActiveId] = useState("s1");
  const [collapsed, setCollapsed] = useState(false);
  const [provider, setProvider] = useState("cloud");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [planStep, setPlanStep] = useState(1);
  const [devMode, setDevMode] = useState(false);
  const [panelVisibility, setPanelVisibility] = useState({
    leftRail: true,
    auxPane: true,
    fileTree: true,
    planTracker: true,
  });
  const timerRef = useRef(null);

  function togglePanel(key) {
    setPanelVisibility((p) => ({ ...p, [key]: !p[key] }));
  }

  const session = SESSIONS.find((s) => s.id === activeId);

  function switchMode(next) {
    if (next === mode) return;
    setFading(true);
    setTimeout(() => {
      setMode(next);
      setFading(false);
    }, 150);
  }

  useEffect(() => {
    clearInterval(timerRef.current);
    if (mode !== "agent") return;
    setPlanStep(1);
    timerRef.current = setInterval(() => {
      setPlanStep((p) => (p < 4 ? p + 1 : p));
    }, 1300);
    return () => clearInterval(timerRef.current);
  }, [mode, activeId]);

  return (
    <div className="h-screen w-full flex flex-col overflow-hidden" style={{ background: C.bg, fontFamily: F_UI }}>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;600;700&family=IBM+Plex+Mono:wght@400;500;600&display=swap');
        @keyframes pulseGlow { 0%,100% { box-shadow: 0 0 0 0 rgba(196,67,43,0.55);} 50% { box-shadow: 0 0 0 6px rgba(196,67,43,0);} }
        .pulse-glow { animation: pulseGlow 1.6s ease-in-out infinite; }
        @keyframes fadeInUp { from { opacity:0; transform: translateY(4px);} to { opacity:1; transform: translateY(0);} }
        .fade-in-up { animation: fadeInUp 220ms ease-out; }
        ::-webkit-scrollbar { width: 8px; height: 8px; }
        ::-webkit-scrollbar-thumb { background: ${C.borderLt}; border-radius: 4px; }
        ::-webkit-scrollbar-track { background: transparent; }
      `}</style>

      <TopRail
        mode={mode}
        setMode={switchMode}
        provider={provider}
        setProvider={setProvider}
        onPalette={() => setPaletteOpen(true)}
        onSettings={() => setSettingsOpen(true)}
        panelVisibility={panelVisibility}
        onTogglePanel={togglePanel}
        devMode={devMode}
      />

      {settingsOpen ? (
        <SettingsView onClose={() => setSettingsOpen(false)} devMode={devMode} setDevMode={setDevMode} />
      ) : (
        <div className="flex flex-1 min-h-0">
          {panelVisibility.leftRail && (
            <LeftRail
              collapsed={collapsed}
              setCollapsed={setCollapsed}
              activeId={activeId}
              setActiveId={setActiveId}
              onHide={() => togglePanel("leftRail")}
            />
          )}

          <div
            className="flex-1 min-w-0"
            style={{
              opacity: fading ? 0 : 1,
              transform: fading ? "scale(0.985)" : "scale(1)",
              transition: "opacity 180ms ease, transform 180ms ease",
            }}
          >
            {mode === "agent" && (
              <AgentView
                session={session}
                planStep={planStep}
                provider={provider}
                panelVisibility={panelVisibility}
                onTogglePanel={togglePanel}
              />
            )}
            {mode === "editor" && (
              <EditorView provider={provider} panelVisibility={panelVisibility} onTogglePanel={togglePanel} />
            )}
            {mode === "design" && <DesignView />}
            {mode === "test" && <TestView />}
            {mode === "ops" && <OpsView />}
            {mode === "data" && <DataView />}
            {mode === "manage" && <ManageView />}
          </div>

          {panelVisibility.auxPane && (
            <AuxPane session={session} provider={provider} onHide={() => togglePanel("auxPane")} />
          )}
        </div>
      )}

      {paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} />}
    </div>
  );
}
