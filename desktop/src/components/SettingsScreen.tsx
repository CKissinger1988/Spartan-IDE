import React, { useCallback, useEffect, useState } from "react";

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

interface Settings {
  gpu_offload: GpuOffloadSettings;
  leo_approval_mode: LeoApprovalMode;
  leo_provider: LeoProviderSettings;
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
 * The wgpu shell's own "Check for Updates" row (§75.49, `spartan-updater`)
 * is real but not wired into this screen this pass -- `spartan-backend`
 * has no `spartan-updater` dependency yet, a real, separate, named
 * follow-up rather than attempted under this pass's own time
 * constraints.
 */
export default function SettingsScreen(): React.ReactElement {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

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

  const save = useCallback(
    (overrides: Partial<Pick<Settings, "gpu_offload" | "leo_approval_mode" | "leo_provider">>) => {
      if (!settings) return;
      setSaving(true);
      const next: Settings = { ...settings, ...overrides };
      window.spartan
        .call("settings_set", {
          gpu_enabled: next.gpu_offload.enabled,
          gpu_layers: next.gpu_offload.layers ?? undefined,
          leo_approval_mode: next.leo_approval_mode,
          leo_provider: next.leo_provider,
        })
        .then((result) => setSettings(result as Settings))
        .catch((e: Error) => setError(e.message))
        .finally(() => setSaving(false));
    },
    [settings]
  );

  if (error) {
    return <div className="settings-screen mono">{error}</div>;
  }
  if (!settings) {
    return <div className="settings-screen mono">Loading settings…</div>;
  }

  const { enabled, layers } = settings.gpu_offload;

  return (
    <div className="settings-screen">
      <div className="settings-section-label mono">Local Model — GPU Offload</div>
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
    </div>
  );
}
