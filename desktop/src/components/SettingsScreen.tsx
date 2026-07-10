import React, { useCallback, useEffect, useState } from "react";

interface GpuOffloadSettings {
  enabled: boolean;
  layers: number | null;
}

interface Settings {
  gpu_offload: GpuOffloadSettings;
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
 * A deliberate, named v1 scope cut: only GPU offload is exposed here,
 * matching `spartan-settings`'s own current real scope (§42's much
 * larger settings taxonomy has no other real backing store yet). The
 * wgpu shell's own "Check for Updates" row (§75.49, `spartan-updater`)
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
    (next: GpuOffloadSettings) => {
      setSaving(true);
      window.spartan
        .call("settings_set", {
          gpu_enabled: next.enabled,
          gpu_layers: next.layers ?? undefined,
        })
        .then((result) => setSettings(result as Settings))
        .catch((e: Error) => setError(e.message))
        .finally(() => setSaving(false));
    },
    []
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
            onChange={(e) => save({ enabled: e.target.checked, layers })}
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
            save({ enabled, layers: v === "auto" ? null : Number(v) });
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
    </div>
  );
}
