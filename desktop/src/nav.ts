// Real navigation model for the desktop shell's information architecture
// -- three grouped sections (Workspace / Build / Platform), matching the
// IA researched from OptimiLabs/velocity (AGPL-3.0; concepts only, no
// code copied -- see docs/architecture-spec.md §75.60 for the full,
// explicit licensing discussion this structure came out of). Spartan's
// own pre-existing features (Editor, Design) are slotted into this same
// structure rather than replacing it, per the user's own explicit
// "with all of our features, functions, additions, and integrations
// added" instruction.

export type ScreenId =
  | "editor"
  | "console"
  | "sessions"
  | "review"
  | "analytics"
  | "usage"
  | "design"
  | "agents"
  | "containers"
  | "workflows"
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
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV: NavGroup[] = [
  {
    label: "Workspace",
    items: [
      { id: "editor", label: "Editor" },
      { id: "console", label: "Console" },
      { id: "sessions", label: "Sessions" },
      { id: "review", label: "Review" },
      { id: "analytics", label: "Analytics" },
      { id: "usage", label: "Usage" },
    ],
  },
  {
    label: "Build",
    items: [
      { id: "design", label: "Design" },
      { id: "agents", label: "Agents" },
      { id: "containers", label: "Dev Containers" },
      { id: "workflows", label: "Workflows" },
      { id: "skills", label: "Skills" },
      { id: "commands", label: "Commands" },
      { id: "hooks", label: "Hooks" },
      { id: "mcp", label: "MCP" },
      { id: "routing", label: "Routing" },
    ],
  },
  {
    label: "Platform",
    items: [
      { id: "models", label: "Models" },
      { id: "plugins", label: "Plugins" },
      { id: "marketplace", label: "Marketplace" },
      { id: "settings", label: "Settings" },
    ],
  },
];

/** Real, honest per-screen descriptions of what's real vs. not-yet-wired
 * -- shown by `Placeholder.tsx` for every screen that isn't `editor` or
 * `workflows` yet. Named specifically, not a generic "coming soon", so
 * each gap is traceable to real, scoped future work. */
export const SCREEN_NOTES: Partial<Record<ScreenId, string>> = {
  review: "Session comparison view -- depends on Sessions existing first.",
  analytics: "Usage/cost/latency tracking -- depends on real session execution data existing first.",
  usage: "Per-provider usage tracking -- same dependency as Analytics.",
  agents:
    "Leo's real chat/plan/approve loop now lives in the persistent right-docked panel (LeoChatPanel.tsx, always visible, not this nav screen) -- see §75.61. This screen is reserved for real future agent *configuration* (approval mode, model selection per task) once that's built, not chat.",
  workflows: "A real, working node-graph canvas -- see WorkflowsScreen.tsx.",
  skills: "Not yet designed for this shell.",
  commands: "Not yet designed for this shell.",
  hooks: "Not yet designed for this shell.",
  mcp: "Not yet designed for this shell.",
  routing: "Depends on Sessions/Agents existing first to have real routing decisions to visualize.",
  models:
    "Real ModelProvider trait, OllamaProvider, ClaudeProvider, and LiteLLMProvider already exist in spartan-model. Not yet exposed via spartan-backend's IPC protocol or this shell's UI.",
  plugins: "Real WASM plugin host (spartan-plugin-host) exists. Not yet exposed to this shell.",
  marketplace: "Not yet designed for this shell.",
};
