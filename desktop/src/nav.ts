// Real navigation model for the desktop shell's information architecture
// -- three grouped sections (Workspace / Build / Platform), matching the
// IA researched from OptimiLabs/velocity (AGPL-3.0; concepts only, no
// code copied -- see docs/architecture-spec.md §75.60 for the full,
// explicit licensing discussion this structure came out of). Spartan's
// own pre-existing features (Editor, Design) are slotted into this same
// structure rather than replacing it, per the user's own explicit
// "with all of our features, functions, additions, and integrations
// added" instruction.
//
// Design is the visual GUI Builder: real JSX/TSX AST edits plus a sandboxed
// live preview, kept behind the same explicit desktop-only boundary.

export type ScreenId =
  | "editor"
  | "device-preview"
  | "web-suite"
  | "console"
  | "sessions"
  | "review"
  | "analytics"
  | "usage"
  | "agents"
  | "containers"
  | "workflows"
  | "design"
  | "skills"
  | "commands"
  | "hooks"
  | "mcp"
  | "routing"
  | "models"
  | "plugins"
  | "marketplace"
  | "settings";

export interface NavItem {
  id: ScreenId;
  label: string;
  /** Compact monochrome glyph shown when the navigation rail is collapsed. */
  icon: string;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV: NavGroup[] = [
  {
    label: "Workspace",
    items: [
      { id: "editor", label: "Editor", icon: "⌘" },
      { id: "device-preview", label: "Device Preview", icon: "▣" },
      { id: "web-suite", label: "Web Studio", icon: "◇" },
      { id: "console", label: "Console", icon: ">_" },
      { id: "sessions", label: "Sessions", icon: "◷" },
      { id: "review", label: "Review", icon: "✓" },
      { id: "analytics", label: "Analytics", icon: "▥" },
      { id: "usage", label: "Usage", icon: "◉" },
    ],
  },
  {
    label: "Build",
    items: [
      { id: "agents", label: "Agents", icon: "✦" },
      { id: "containers", label: "Dev Containers", icon: "□" },
      { id: "workflows", label: "Workflows", icon: "⌘" },
      { id: "design", label: "GUI Builder", icon: "✥" },
      { id: "skills", label: "Skills", icon: "✧" },
      { id: "commands", label: "Commands", icon: "⚑" },
      { id: "hooks", label: "Hooks", icon: "⌁" },
      { id: "mcp", label: "MCP", icon: "⬡" },
      { id: "routing", label: "Routing", icon: "↗" },
    ],
  },
  {
    label: "Platform",
    items: [
      { id: "models", label: "Models", icon: "◎" },
      { id: "plugins", label: "Plugins", icon: "⚙" },
      { id: "marketplace", label: "Marketplace", icon: "◇" },
      { id: "settings", label: "Settings", icon: "⚙" },
    ],
  },
];

/** Real, honest per-screen descriptions of what's real vs. not-yet-wired
 * -- used as contextual metadata by workspace-backed destinations. Named
 * specifically, not as generic "coming soon" text, so
 * each gap is traceable to real, scoped future work. */
export const SCREEN_NOTES: Partial<Record<ScreenId, string>> = {
  review: "Session comparison view -- depends on Sessions existing first.",
  analytics: "Usage/cost/latency tracking -- depends on real session execution data existing first.",
  usage: "Per-provider usage tracking -- same dependency as Analytics.",
  agents:
    "Leo's real chat/plan/approve loop now lives in the persistent right-docked panel (LeoChatPanel.tsx, always visible, not this nav screen) -- see §75.61. This screen is reserved for real future agent *configuration* (approval mode, model selection per task) once that's built, not chat.",
  workflows: "A real, working node-graph canvas -- see WorkflowsScreen.tsx.",
  routing: "Depends on Sessions/Agents existing first to have real routing decisions to visualize.",
  plugins: "Project plugin resources and the WASM plugin host are available from this workspace tool.",
};
