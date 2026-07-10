import React, { useCallback, useEffect, useState } from "react";
import { applyReduceMotion } from "../reduceMotion";
import pkg from "../../package.json";

interface GpuOffloadSettings {
  enabled: boolean;
  layers: number | null;
}

type LeoApprovalMode = "ManualEveryStep" | "AutoApproveSafe";

type LeoProviderKind = "Ollama" | "Claude" | "LiteLLM";

interface LeoProviderSettings {
  kind: LeoProviderKind;
  model: string;
}

interface EditorSettings {
  font_size: number;
  tab_size: number;
  word_wrap: boolean;
}

interface AppearanceSettings {
  reduce_motion: boolean;
}

interface Settings {
  gpu_offload: GpuOffloadSettings;
  leo_approval_mode: LeoApprovalMode;
  leo_provider: LeoProviderSettings;
  editor: EditorSettings;
  appearance: AppearanceSettings;
  onboarding_completed: boolean;
}

/** Real, sensible default model per provider kind -- shown the moment the
 * user switches kind in the UI, before they've typed anything of their
 * own; matches each provider's own already-established test/default
 * precedent in `spartan-model`/`spartan-settings`. */
const DEFAULT_MODEL_FOR_KIND: Record<LeoProviderKind, string> = {
  Ollama: "llama3.1:8b",
  Claude: "claude-3-5-sonnet-latest",
  LiteLLM: "gpt-4o",
};

interface UpdateCheckCategories {
  language_definitions_changed: boolean;
  leo_changed: boolean;
  other_changed: boolean;
}

interface UpdateCheckResult {
  current_commit: string;
  latest_commit: string;
  up_to_date: boolean;
  categories: UpdateCheckCategories;
}

type UpdateCheckDisplay =
  | { kind: "not_checked" }
  | { kind: "checking" }
  | { kind: "ready"; result: UpdateCheckResult }
  | { kind: "failed"; error: string };

function shortCommit(commit: string): string {
  return commit.slice(0, 7);
}

/** Real, live text for the "Check for Updates" row's own current state --
 * a real category breakdown on a real update, not just "update
 * available." Mirrors the original wgpu shell's own `update_check_line`
 * (`settings_panel.rs`, §75.49) line for line. */
function updateCheckLine(state: UpdateCheckDisplay): string {
  switch (state.kind) {
    case "not_checked":
      return "Not checked yet";
    case "checking":
      return "Checking for updates…";
    case "ready": {
      const { result } = state;
      if (result.up_to_date) {
        return `Up to date (${shortCommit(result.current_commit)})`;
      }
      const parts: string[] = [];
      if (result.categories.language_definitions_changed) parts.push("language definitions");
      if (result.categories.leo_changed) parts.push("Leo/agent core");
      if (result.categories.other_changed) parts.push("other IDE code");
      const what = parts.length > 0 ? parts.join(", ") : "changes";
      return `Update available: ${what} (${shortCommit(result.current_commit)} → ${shortCommit(result.latest_commit)})`;
    }
    case "failed":
      return `Update check failed: ${state.error}`;
  }
}

/**
 * Real Settings screen for the Electron shell (§42, user-requested "GPU
 * offloading toggle and amount to offload selector"), the first half of
 * the "Git panel + Settings screen" priority item -- ports the original
 * wgpu shell's own `settings_panel.rs` (§75.48) to this shell's real
 * `spartan-backend` IPC methods (`settings_get`/`settings_set`), which
 * wrap `spartan_settings` directly so both shells persist to and read
 * from the exact same real `~/.spartan/settings.json`.
 *
 * Since §75.69 this screen also exposes Leo's real approval mode --
 * `ManualEveryStep` (the real, non-negotiable default: every real tool
 * call needs an explicit human click) or `AutoApproveSafe` (a real
 * `read_file`/`search_files`/`list_directory` call runs immediately,
 * server-side, without a UI round trip; `edit_file`/`run_terminal` are
 * never auto-approved by either mode, matching §9's own non-negotiable
 * rule -- `Agent::may_auto_execute` is the one real gate this setting
 * feeds into, unchanged).
 *
 * §75.72 closes the gap §75.49/§75.65 both named: the wgpu shell's own
 * "Check for Updates" row (`spartan-updater`) is now wired into this
 * shell too, via `spartan-backend`'s real `check_for_updates` IPC method
 * -- a real, possibly-slow HTTPS call, so it follows the exact same
 * "immediate ack + later unprompted event" pattern the Leo chat panel
 * already established for `leo_start_task`.
 */
export default function SettingsScreen(): React.ReactElement {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [updateCheck, setUpdateCheck] = useState<UpdateCheckDisplay>({ kind: "not_checked" });

  const refresh = useCallback(() => {
    window.spartan
      .call("settings_get", {})
      .then((result) => {
        setSettings(result as Settings);
        setError(null);
      })
      .catch((e: Error) => setError(e.message));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "update_check_result") {
        setUpdateCheck({ kind: "ready", result: data as UpdateCheckResult });
      } else if (event === "update_check_failed") {
        setUpdateCheck({ kind: "failed", error: (data as { error: string }).error });
      }
    });
    return unsubscribe;
  }, []);

  const checkForUpdates = useCallback(() => {
    setUpdateCheck({ kind: "checking" });
    window.spartan.call("check_for_updates", {}).catch((e: Error) => {
      setUpdateCheck({ kind: "failed", error: e.message });
    });
  }, []);

  const save = useCallback(
    (overrides: Partial<Settings>) => {
      if (!settings) return;
      setSaving(true);
      const next: Settings = { ...settings, ...overrides };
      // Real §75.76 "reduce motion" optimistic UI update -- applied
      // immediately rather than waiting for the round trip, so the
      // animations actually stop/start the instant the checkbox is
      // toggled.
      if (overrides.appearance) {
        applyReduceMotion(overrides.appearance.reduce_motion);
      }
      window.spartan
        .call("settings_set", {
          gpu_enabled: next.gpu_offload.enabled,
          gpu_layers: next.gpu_offload.layers ?? undefined,
          leo_approval_mode: next.leo_approval_mode,
          leo_provider: next.leo_provider,
          editor: next.editor,
          appearance: next.appearance,
          onboarding_completed: next.onboarding_completed,
        })
        .then((result) => setSettings(result as Settings))
        .catch((e: Error) => setError(e.message))
        .finally(() => setSaving(false));
    },
    [settings]
  );

  const openCrashReportsFolder = useCallback(() => {
    window.spartan.openCrashReportsFolder?.().catch((e: Error) => setError(e.message));
  }, []);

  const openRepositoryPage = useCallback(() => {
    window.spartan.openRepositoryPage?.().catch((e: Error) => setError(e.message));
  }, []);

  if (error) {
    return <div className="settings-screen mono">{error}</div>;
  }
  if (!settings) {
    return <div className="settings-screen mono">Loading settings…</div>;
  }

  const { enabled, layers } = settings.gpu_offload;

  return (
    <div className="settings-screen">
      <div className="settings-section-label mono">Editor</div>
      <div className="settings-row">
        <label className="settings-label mono">Font size</label>
        <input
          className="settings-select mono"
          type="number"
          min={9}
          max={32}
          disabled={saving}
          value={settings.editor.font_size}
          onChange={(e) => {
            const font_size = Number(e.target.value) || settings.editor.font_size;
            save({ editor: { ...settings.editor, font_size } });
          }}
          style={{ width: 64 }}
        />
      </div>
      <div className="settings-row">
        <label className="settings-label mono">Tab size</label>
        <select
          className="settings-select mono"
          disabled={saving}
          value={settings.editor.tab_size}
          onChange={(e) =>
            save({ editor: { ...settings.editor, tab_size: Number(e.target.value) } })
          }
        >
          {[2, 4, 8].map((n) => (
            <option key={n} value={n}>
              {n} spaces
            </option>
          ))}
        </select>
      </div>
      <div className="settings-row">
        <label className="settings-label mono">
          <input
            type="checkbox"
            checked={settings.editor.word_wrap}
            disabled={saving}
            onChange={(e) =>
              save({ editor: { ...settings.editor, word_wrap: e.target.checked } })
            }
          />
          {" "}Word wrap
        </label>
      </div>
      <div className="settings-note mono">
        Font size, tab size (also used by the Tab key), and word wrap apply the next time a file
        is opened — an already-open tab is unaffected until you switch to it or reopen it.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Appearance
      </div>
      <div className="settings-row">
        <label className="settings-label mono">
          <input
            type="checkbox"
            checked={settings.appearance.reduce_motion}
            disabled={saving}
            onChange={(e) =>
              save({ appearance: { reduce_motion: e.target.checked } })
            }
          />
          {" "}Reduce motion
        </label>
      </div>
      <div className="settings-note mono">
        Spartan's theme uses glow pulses and a scan-line sweep on a few real-time status
        indicators (Leo's state badge, the running Dev Containers badge, the sidebar brand). This
        turns all of it off instantly, everywhere, without changing color or layout.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Local Model — GPU Offload
      </div>
      <div className="settings-row">
        <label className="settings-label mono">
          <input
            type="checkbox"
            checked={enabled}
            disabled={saving}
            onChange={(e) => save({ gpu_offload: { enabled: e.target.checked, layers } })}
          />
          {" "}GPU offloading enabled
        </label>
      </div>
      <div className="settings-row">
        <label className="settings-label mono">GPU layers to offload</label>
        <select
          className="settings-select mono"
          disabled={!enabled || saving}
          value={layers === null ? "auto" : String(layers)}
          onChange={(e) => {
            const v = e.target.value;
            save({ gpu_offload: { enabled, layers: v === "auto" ? null : Number(v) } });
          }}
        >
          <option value="auto">Auto</option>
          {[0, 1, 2, 4, 8, 16, 24, 32, 48, 64, 96, 128].map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </div>
      <div className="settings-note mono">
        Disabled forces pure CPU inference. Enabled with no explicit count lets Ollama auto-offload.
        Settings are shared with the original wgpu shell via the same real ~/.spartan/settings.json.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Leo — Approval Mode
      </div>
      <div className="settings-row">
        <label className="settings-label mono">Tool call approval</label>
        <select
          className="settings-select mono"
          disabled={saving}
          value={settings.leo_approval_mode}
          onChange={(e) => save({ leo_approval_mode: e.target.value as LeoApprovalMode })}
        >
          <option value="ManualEveryStep">Manual — approve every step</option>
          <option value="AutoApproveSafe">Auto-approve safe reads (search / list / read)</option>
        </select>
      </div>
      <div className="settings-note mono">
        Manual mode requires an explicit click before any real tool call runs. Auto-approve safe
        reads still always requires approval for edit_file and run_terminal — read-only exploration
        (search_files/list_directory/read_file) runs immediately instead.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Leo — Model Provider
      </div>
      <div className="settings-row">
        <label className="settings-label mono">Provider</label>
        <select
          className="settings-select mono"
          disabled={saving}
          value={settings.leo_provider.kind}
          onChange={(e) => {
            const kind = e.target.value as LeoProviderKind;
            save({ leo_provider: { kind, model: DEFAULT_MODEL_FOR_KIND[kind] } });
          }}
        >
          <option value="Ollama">Ollama (local)</option>
          <option value="Claude">Claude (Anthropic API)</option>
          <option value="LiteLLM">LiteLLM (local proxy → cloud backends)</option>
        </select>
      </div>
      <div className="settings-row">
        <label className="settings-label mono">Model</label>
        <input
          className="settings-select mono"
          type="text"
          disabled={saving}
          defaultValue={settings.leo_provider.model}
          key={settings.leo_provider.kind}
          onBlur={(e) => {
            const model = e.target.value.trim();
            if (model && model !== settings.leo_provider.model) {
              save({ leo_provider: { kind: settings.leo_provider.kind, model } });
            }
          }}
        />
      </div>
      <div className="settings-note mono">
        Ollama runs fully local (GPU offload above applies). Claude reads ANTHROPIC_API_KEY from
        the environment — no key storage exists in this settings screen yet. LiteLLM routes
        through a local proxy at localhost:4000 to whichever cloud backend it's configured for.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Updates
      </div>
      <div className="settings-row">
        <button
          className="settings-button mono"
          disabled={updateCheck.kind === "checking"}
          onClick={checkForUpdates}
        >
          Check for Updates
        </button>
        <span className="settings-update-status mono">{updateCheckLine(updateCheck)}</span>
      </div>
      <div className="settings-note mono">
        A real, live check against this project's own GitHub repository for whether a newer
        build exists, categorized by language definitions, Leo/agent core, or other IDE code.
        No download, install, or restart of any kind — this only tells you something is
        available so you can act on it yourself.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Privacy &amp; Diagnostics
      </div>
      <div className="settings-row">
        <button className="settings-button mono" onClick={openCrashReportsFolder}>
          Open Crash Reports Folder
        </button>
      </div>
      <div className="settings-note mono">
        Spartan has no telemetry of any kind — nothing about your usage is ever sent anywhere.
        A crash panic is caught locally, has any credential-shaped text redacted, and is written
        as a plain JSON file under ~/.spartan/crashes/. Nothing in that folder is ever uploaded
        automatically; deleting its contents is safe at any time.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Keyboard Shortcuts
      </div>
      <div className="settings-note mono">
        Ctrl/Cmd+S save · Ctrl/Cmd+Z undo · Ctrl/Cmd+Y or Ctrl/Cmd+Shift+Z redo · Ctrl/Cmd+G
        toggle Files/Git sidebar · Tab inserts the configured tab size · Ctrl/Cmd+Enter (in Leo's
        task box) submit a task to Leo.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        About
      </div>
      <div className="settings-note mono">
        Spartan IDE v{pkg.version} — a from-scratch, agent-first desktop IDE. Real Rust core
        (rope buffer, tree-sitter, in-house LSP/DAP, git), driven by this real Electron + React
        shell over a local IPC service. No VS Code, Monaco, or CodeMirror code is forked or
        vendored anywhere in this repository.
      </div>
      <div className="settings-row">
        <button className="settings-button mono" onClick={openRepositoryPage}>
          View on GitHub
        </button>
      </div>
    </div>
  );
}
