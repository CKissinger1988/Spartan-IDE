import React, { useCallback, useEffect, useState } from "react";
import { applyReduceMotion } from "../reduceMotion";
import { applyTheme, THEME_LABELS, type ThemeName } from "../applyTheme";
import { applyFontFamily } from "../applyFontFamily";
import type { UserSnippet } from "../snippets";
import pkg from "../../package.json";

interface GpuOffloadSettings {
  enabled: boolean;
  layers: number | null;
}

type LeoApprovalMode = "ManualEveryStep" | "AutoApproveSafe";

type LeoProviderKind = "Ollama" | "Claude" | "LiteLLM" | "LlamaCpp" | "LmStudio";

interface LeoProviderSettings {
  kind: LeoProviderKind;
  model: string;
}

interface EditorSettings {
  font_size: number;
  tab_size: number;
  word_wrap: boolean;
  font_family: string | null;
  format_on_save: boolean;
}

interface AppearanceSettings {
  reduce_motion: boolean;
  theme: ThemeName;
}

interface CrashReportingSettings {
  upload_endpoint: string | null;
}

interface Settings {
  gpu_offload: GpuOffloadSettings;
  leo_approval_mode: LeoApprovalMode;
  leo_provider: LeoProviderSettings;
  editor: EditorSettings;
  appearance: AppearanceSettings;
  crash_reporting: CrashReportingSettings;
  onboarding_completed: boolean;
  /** Real task #265 -- the shell command Leo runs during `Verifying`
   * before declaring a task `Done`. `null` (the default) keeps
   * `Verifying` a real, momentary, always-passing waypoint. */
  leo_verify_command: string | null;
  /** Real user-defined snippets (the follow-up the curated-snippets pass
   * named) -- persisted as `spartan_settings::UserSnippet` objects and
   * honored by both shells' `findSnippet` on top of the curated
   * `SNIPPETS` table. */
  user_snippets: UserSnippet[];
}

interface CrashReportEntry {
  filename: string;
  report: { unix_timestamp: number; message: string; location: string | null };
}

type UploadStatus =
  | { kind: "idle" }
  | { kind: "uploading" }
  | { kind: "done"; status: number }
  | { kind: "failed"; error: string };

/** Real, sensible default model per provider kind -- shown the moment the
 * user switches kind in the UI, before they've typed anything of their
 * own; matches each provider's own already-established test/default
 * precedent in `spartan-model`/`spartan-settings`. */
const DEFAULT_MODEL_FOR_KIND: Record<LeoProviderKind, string> = {
  Ollama: "llama3.1:8b",
  Claude: "claude-3-5-sonnet-latest",
  LiteLLM: "gpt-4o",
  // LM Studio serves whichever model it has loaded; this is a placeholder the
  // user replaces with their loaded model's id (or leaves as a reminder).
  LmStudio: "local-model",
  // Real, deliberate empty default -- unlike the other providers'
  // real, valid model-name defaults, there is no universal real .gguf
  // path this could point at; the user must Browse to (or type) a real
  // local file.
  LlamaCpp: "",
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

/** Real shape of `model_status_json()` (`crates/spartan-backend`) -- either
 * a configured provider (with a real live health probe) or an honest
 * construction error, never a fabricated success. */
interface ModelStatusResult {
  configured: boolean;
  kind: string;
  model?: string;
  provider_id?: string;
  is_local?: boolean;
  context_window?: number;
  supports_native_tool_calling?: boolean;
  health?: string;
  fallback_count?: number;
  error?: string;
}

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
  const [updateAvailableVersion, setUpdateAvailableVersion] = useState<string | null>(null);
  const [updateDownloadedVersion, setUpdateDownloadedVersion] = useState<string | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [modelStatus, setModelStatus] = useState<ModelStatusResult | null>(null);
  const [modelStatusChecking, setModelStatusChecking] = useState(false);
  const [crashReports, setCrashReports] = useState<CrashReportEntry[]>([]);
  const [crashReportsError, setCrashReportsError] = useState<string | null>(null);
  const [uploadStatus, setUploadStatus] = useState<Record<string, UploadStatus>>({});
  const [snippetDraft, setSnippetDraft] = useState<UserSnippet[]>([]);
  const [snippetError, setSnippetError] = useState<string | null>(null);

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

  const refreshCrashReports = useCallback(() => {
    window.spartan
      .call("crash_reports_list", {})
      .then((result) => {
        setCrashReports((result as { reports: CrashReportEntry[] }).reports);
        setCrashReportsError(null);
      })
      .catch((e: Error) => setCrashReportsError(e.message));
  }, []);

  useEffect(() => {
    refreshCrashReports();
  }, [refreshCrashReports]);

  // Keep the snippet editor in sync with the real persisted list: it must
  // reflect a fresh `settings_get` and (crucially) the response of every
  // `settings_set`, so a snippet the user just saved is still there when
  // the section re-renders. Editing fields never touches `settings`
  // directly -- the working copy lives in `snippetDraft` and only reaches
  // the backend when "Save snippets" is pressed.
  useEffect(() => {
    setSnippetDraft(settings?.user_snippets ?? []);
  }, [settings?.user_snippets]);

  const setSnippetField = useCallback(
    (index: number, field: keyof UserSnippet, value: string) => {
      setSnippetError(null);
      setSnippetDraft((prev) => prev.map((s, i) => (i === index ? { ...s, [field]: value } : s)));
    },
    []
  );

  const removeSnippet = useCallback((index: number) => {
    setSnippetError(null);
    setSnippetDraft((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const addSnippet = useCallback(() => {
    setSnippetError(null);
    setSnippetDraft((prev) => [...prev, { lang_id: "", prefix: "", body: "", description: "" }]);
  }, []);

  useEffect(() => {
    const unsub1 = window.spartan.onEvent((event, data) => {
      if (event === "update_check_result") {
        setUpdateCheck({ kind: "ready", result: data as UpdateCheckResult });
      } else if (event === "update_check_failed") {
        setUpdateCheck({ kind: "failed", error: (data as { error: string }).error });
      }
    });
    // Real auto-update event subscriptions (§75.49 apply path):
    // electron-updater fires these from the main process via preload.
    const unsub2 = window.spartan.onUpdateAvailable?.((info) => {
      setUpdateAvailableVersion(info.version);
      setUpdateError(null);
    }) ?? (() => {});
    const unsub3 = window.spartan.onUpdateDownloadProgress?.((info) => {
      setDownloadProgress(Math.round(info.percent));
    }) ?? (() => {});
    const unsub4 = window.spartan.onUpdateDownloaded?.((info) => {
      setUpdateDownloadedVersion(info.version);
      setDownloading(false);
      setUpdateAvailableVersion(null);
    }) ?? (() => {});
    const unsub5 = window.spartan.onUpdateError?.((info) => {
      setUpdateError(info.message);
      setDownloading(false);
    }) ?? (() => {});
    return () => { unsub1(); unsub2(); unsub3(); unsub4(); unsub5(); };
  }, []);

  const checkForUpdates = useCallback(() => {
    setUpdateCheck({ kind: "checking" });
    window.spartan.call("check_for_updates", {}).catch((e: Error) => {
      setUpdateCheck({ kind: "failed", error: e.message });
    });
  }, []);

  // Real auto-update download handler (§75.49 apply path):
  // triggers electron-updater's download flow via the main process IPC.
  const downloadUpdate = useCallback(() => {
    setDownloading(true);
    setDownloadProgress(0);
    setUpdateError(null);
    window.spartan.call("download_update", {}).catch((e: Error) => {
      setUpdateError(e.message);
      setDownloading(false);
    });
  }, []);

  // Real auto-update install handler: triggers quit-and-restart with the
  // downloaded update. The main process calls autoUpdater.quitAndInstall().
  const installUpdate = useCallback(() => {
    window.spartan.call("install_update", {}).catch((e: Error) => {
      setUpdateError(e.message);
    });
  }, []);

  // Real, synchronous (unlike check_for_updates above) -- model_status_json
  // itself performs the live health probe before returning, so there's no
  // async event to subscribe to, matching this method's own real backend
  // shape.
  const checkModelStatus = useCallback(() => {
    setModelStatusChecking(true);
    window.spartan
      .call("model_status", {})
      .then((result) => setModelStatus(result as ModelStatusResult))
      .catch((e: Error) => setModelStatus({ configured: false, kind: "?", error: e.message }))
      .finally(() => setModelStatusChecking(false));
  }, []);

  const save = useCallback(
    (overrides: Partial<Settings>) => {
      if (!settings) return;
      setSaving(true);
      const next: Settings = { ...settings, ...overrides };
      // Real §75.76 "reduce motion" / §75.93 theme+font optimistic UI
      // update -- applied immediately rather than waiting for the round
      // trip, so the effect is visible the instant the control changes.
      if (overrides.appearance) {
        applyReduceMotion(overrides.appearance.reduce_motion);
        applyTheme(overrides.appearance.theme);
      }
      if (overrides.editor) {
        applyFontFamily(overrides.editor.font_family);
      }
      window.spartan
        .call("settings_set", {
          gpu_enabled: next.gpu_offload.enabled,
          gpu_layers: next.gpu_offload.layers ?? undefined,
          leo_approval_mode: next.leo_approval_mode,
          leo_provider: next.leo_provider,
          editor: next.editor,
          appearance: next.appearance,
          crash_reporting: next.crash_reporting,
          onboarding_completed: next.onboarding_completed,
          // Real nested-Option patch shape (task #265): omit entirely to
          // leave the current value untouched (this key is only ever
          // included in `overrides` when the verify-command row itself
          // changed); an empty string clears it, matching the backend's
          // own trim-then-empty-means-None parse.
          ...(overrides.leo_verify_command !== undefined
            ? { leo_verify_command: overrides.leo_verify_command ?? "" }
            : {}),
          // Real full-list patch shape: `Some(list)` replaces the whole
          // `Settings.user_snippets` list (the snippet editor sends the
          // entire edited list every save); omitted entirely when no
          // snippet row changed, leaving the current list untouched.
          ...(overrides.user_snippets !== undefined
            ? { user_snippets: overrides.user_snippets }
            : {}),
        })
        .then((result) => setSettings(result as Settings))
        .catch((e: Error) => setError(e.message))
        .finally(() => setSaving(false));
    },
    [settings]
  );

  const saveSnippets = useCallback(() => {
    // Real, honest client-side validation mirroring `spartan-backend`'s
    // own `settings_set` rules exactly (non-empty `lang_id`/`prefix`/`body`)
    // -- catch it here in the UI instead of letting the backend fail the
    // whole save with a less friendly error.
    const trimmed = snippetDraft.map((s) => ({
      lang_id: s.lang_id.trim(),
      prefix: s.prefix.trim(),
      body: s.body.trim(),
      description: s.description.trim(),
    }));
    const bad = trimmed.findIndex((s) => !s.lang_id || !s.prefix || !s.body);
    if (bad !== -1) {
      const field = !trimmed[bad].lang_id
        ? "language"
        : !trimmed[bad].prefix
          ? "prefix"
          : "body";
      setSnippetError(`Snippet ${bad + 1}: ${field} must not be empty`);
      return;
    }
    setSnippetError(null);
    save({ user_snippets: trimmed });
  }, [snippetDraft, save]);

  const openCrashReportsFolder = useCallback(() => {
    window.spartan.openCrashReportsFolder?.().catch((e: Error) => setError(e.message));
  }, []);

  // Real, explicit, per-report upload -- never automatic (§18, §75.82).
  // Each report gets its own independent status so uploading one doesn't
  // block or misreport the state of any other.
  const uploadReport = useCallback((filename: string) => {
    setUploadStatus((prev) => ({ ...prev, [filename]: { kind: "uploading" } }));
    window.spartan
      .call("crash_report_upload", { filename })
      .then((result) => {
        const status = (result as { status: number }).status;
        setUploadStatus((prev) => ({ ...prev, [filename]: { kind: "done", status } }));
      })
      .catch((e: Error) => {
        setUploadStatus((prev) => ({ ...prev, [filename]: { kind: "failed", error: e.message } }));
      });
  }, []);

  const uploadStatusLine = (status: UploadStatus | undefined): string | null => {
    if (!status || status.kind === "idle") return null;
    if (status.kind === "uploading") return "Uploading…";
    if (status.kind === "done") return `Uploaded (HTTP ${status.status})`;
    return `Failed: ${status.error}`;
  };

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
          defaultValue={settings.editor.font_size}
          key={settings.editor.font_size}
          onBlur={(e) => {
            // Real bug fix, found live by a code-review pass: this used
            // to be a controlled `value`+`onChange` input that called
            // `save()` (which sets `saving=true` synchronously) on every
            // keystroke -- disabling a focused input blurs it, so typing
            // "20" would drop the "0" after the field disabled itself
            // mid-keystroke. Matches the "Model" field below's own
            // already-correct `onBlur`-commit pattern -- and a genuinely
            // empty field on blur (not mid-typing) now keeps the
            // existing value instead of the old falsy-zero fallback
            // silently reverting it.
            const parsed = Number(e.target.value);
            if (Number.isFinite(parsed) && parsed >= 9 && parsed <= 32) {
              save({ editor: { ...settings.editor, font_size: parsed } });
            }
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
      <div className="settings-row">
        <label className="settings-label mono">
          <input
            type="checkbox"
            checked={settings.editor.format_on_save}
            disabled={saving}
            onChange={(e) =>
              save({ editor: { ...settings.editor, format_on_save: e.target.checked } })
            }
          />
          {" "}Format document on save
        </label>
      </div>
      <div className="settings-row">
        <label className="settings-label mono">Font family</label>
        <input
          className="settings-select mono"
          type="text"
          placeholder="JetBrains Mono (bundled default)"
          disabled={saving}
          defaultValue={settings.editor.font_family ?? ""}
          key={settings.editor.font_family ?? ""}
          onBlur={(e) => {
            const trimmed = e.target.value.trim();
            save({
              editor: { ...settings.editor, font_family: trimmed.length > 0 ? trimmed : null },
            });
          }}
          style={{ width: 220 }}
        />
      </div>
      <div className="settings-note mono">
        Font size, tab size (also used by the Tab key), and word wrap apply the next time a file
        is opened — an already-open tab is unaffected until you switch to it or reopen it. Font
        family applies live, everywhere in the app, immediately. Leave it blank to use the real
        bundled JetBrains Mono; a name here must already be installed on this machine (or resolve
        via a real system font-fallback) to actually take visible effect.
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        Appearance
      </div>
      <div className="settings-row">
        <label className="settings-label mono">Theme</label>
        <select
          className="settings-select mono"
          disabled={saving}
          value={settings.appearance.theme}
          onChange={(e) =>
            save({
              appearance: { ...settings.appearance, theme: e.target.value as ThemeName },
            })
          }
        >
          {(Object.keys(THEME_LABELS) as ThemeName[]).map((name) => (
            <option key={name} value={name}>
              {THEME_LABELS[name]}
            </option>
          ))}
        </select>
      </div>
      <div className="settings-row">
        <label className="settings-label mono">
          <input
            type="checkbox"
            checked={settings.appearance.reduce_motion}
            disabled={saving}
            onChange={(e) =>
              save({
                appearance: { ...settings.appearance, reduce_motion: e.target.checked },
              })
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
      <div className="settings-row">
        <label className="settings-label mono">Verification command</label>
        <input
          className="settings-select mono"
          type="text"
          placeholder="e.g. cargo test  (leave blank to skip)"
          disabled={saving}
          defaultValue={settings.leo_verify_command ?? ""}
          key={settings.leo_verify_command ?? ""}
          onBlur={(e) => {
            const cmd = e.target.value.trim();
            if (cmd !== (settings.leo_verify_command ?? "")) {
              save({ leo_verify_command: cmd.length > 0 ? cmd : null });
            }
          }}
          style={{ width: 320 }}
        />
      </div>
      <div className="settings-note mono">
        Runs through the project's real sandbox before Leo declares a task Done (bounded to 120s,
        §264). A non-zero exit marks the task Failed instead — the exact state the Retry button
        recovers from — so a real failing check genuinely feeds Leo's own bounded recovery loop.
        Leave blank to keep Verifying a momentary, always-passing waypoint.
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
          <option value="LmStudio">LM Studio (local server)</option>
          <option value="LlamaCpp">llama.cpp (local, in-process GGUF)</option>
        </select>
      </div>
      <div className="settings-row">
        <label className="settings-label mono">
          {settings.leo_provider.kind === "LlamaCpp" ? "Model file (.gguf)" : "Model"}
        </label>
        <input
          className="settings-select mono"
          type="text"
          placeholder={
            settings.leo_provider.kind === "LlamaCpp" ? "/path/to/model.gguf" : undefined
          }
          disabled={saving}
          defaultValue={settings.leo_provider.model}
          key={`${settings.leo_provider.kind}-${settings.leo_provider.model}`}
          onBlur={(e) => {
            const model = e.target.value.trim();
            if (model && model !== settings.leo_provider.model) {
              save({ leo_provider: { kind: settings.leo_provider.kind, model } });
            }
          }}
          style={{ width: settings.leo_provider.kind === "LlamaCpp" ? 320 : undefined }}
        />
        {settings.leo_provider.kind === "LlamaCpp" && (
          <button
            className="settings-button mono"
            disabled={saving}
            onClick={() => {
              window.spartan
                .pickFile([{ name: "GGUF models", extensions: ["gguf"] }])
                .then((result) => {
                  const r = result as { canceled: boolean; path: string | null };
                  if (!r.canceled && r.path) {
                    save({ leo_provider: { kind: "LlamaCpp", model: r.path } });
                  }
                })
                .catch((e: Error) => setError(e.message));
            }}
          >
            Browse…
          </button>
        )}
      </div>
      <div className="settings-note mono">
        Ollama runs fully local (GPU offload above applies). Claude reads ANTHROPIC_API_KEY from
        the environment — no key storage exists in this settings screen yet. LiteLLM routes
        through a local proxy at localhost:4000 to whichever cloud backend it's configured for.
        llama.cpp runs a real local GGUF model in-process (no separate server, no Ollama install
        required) — point it at a real `.gguf` file on disk. Tool calling is real and native here
        too, via grammar-constrained sampling: the model's output is structurally forced to match
        the tool schema, so Leo's plan/execute loop works fully through it, not just free-text
        completion.
      </div>
      <div className="settings-row">
        <button className="settings-button mono" disabled={modelStatusChecking} onClick={checkModelStatus}>
          {modelStatusChecking ? "Checking…" : "Check Status"}
        </button>
        <span className="settings-update-status mono">
          {modelStatus === null
            ? "not checked yet"
            : modelStatus.configured
              ? `${modelStatus.kind} (${modelStatus.model}) — ${modelStatus.health}${
                  modelStatus.is_local ? ", local" : ""
                }`
              : `not configured: ${modelStatus.error}`}
        </span>
      </div>

      <div className="settings-section-label mono" style={{ marginTop: 28 }}>
        User Snippets
      </div>
      <div className="settings-note mono" style={{ marginTop: 0 }}>
        Type a prefix and press Tab in a file of the matching language to expand a snippet —
        exactly like the built-in ones, and a user snippet for the same language + prefix pair
        overrides the built-in one. Language is the editor's own id for that file type (e.g.
        <span className="mono"> python</span>,<span className="mono"> rust</span>,<span className="mono"> typescript</span>,
        <span className="mono"> go</span>,<span className="mono"> java</span>,<span className="mono"> kotlin</span>). Bodies use the same
        template syntax as the built-in snippets: <span className="mono">{"${1:name}"}</span> for a numbered stop,
        <span className="mono"> $0</span> for the final cursor position.
      </div>
      {snippetDraft.map((snip, i) => (
        <div
          key={i}
          style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 12 }}
        >
          <div className="settings-row" style={{ marginBottom: 0 }}>
            <label className="settings-label mono" style={{ width: 72 }}>
              Language
            </label>
            <input
              className="settings-select mono"
              type="text"
              value={snip.lang_id}
              disabled={saving}
              onChange={(e) => setSnippetField(i, "lang_id", e.target.value)}
              style={{ width: 130 }}
            />
            <label className="settings-label mono" style={{ width: 44 }}>
              Prefix
            </label>
            <input
              className="settings-select mono"
              type="text"
              value={snip.prefix}
              disabled={saving}
              onChange={(e) => setSnippetField(i, "prefix", e.target.value)}
              style={{ width: 120 }}
            />
            <button
              className="settings-button mono"
              disabled={saving}
              onClick={() => removeSnippet(i)}
              title="Remove this snippet"
            >
              Remove
            </button>
          </div>
          <div className="settings-row" style={{ marginBottom: 0 }}>
            <label className="settings-label mono" style={{ width: 72 }}>
              Body
            </label>
            <textarea
              className="settings-select mono"
              value={snip.body}
              disabled={saving}
              onChange={(e) => setSnippetField(i, "body", e.target.value)}
              rows={3}
              spellCheck={false}
              style={{ width: 330, fontFamily: "var(--font-mono)", resize: "vertical" }}
            />
          </div>
          <div className="settings-row" style={{ marginBottom: 0 }}>
            <label className="settings-label mono" style={{ width: 72 }}>
              Description
            </label>
            <input
              className="settings-select mono"
              type="text"
              value={snip.description}
              disabled={saving}
              onChange={(e) => setSnippetField(i, "description", e.target.value)}
              style={{ width: 330 }}
            />
          </div>
        </div>
      ))}
      {snippetDraft.length === 0 && (
        <div className="settings-update-status mono" style={{ marginTop: 12 }}>
          No user snippets defined. Add one to get started.
        </div>
      )}
      {snippetError && (
        <div className="leo-error mono" style={{ marginTop: 8 }}>
          {snippetError}
        </div>
      )}
      <div className="settings-row" style={{ marginBottom: 0, borderBottom: "none" }}>
        <button className="settings-button mono" disabled={saving} onClick={addSnippet}>
          Add snippet
        </button>
        <button
          className="settings-button mono settings-button-primary"
          disabled={saving}
          onClick={saveSnippets}
        >
          Save snippets
        </button>
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
      </div>
      {updateAvailableVersion && (
        <div className="settings-row" style={{ marginTop: 8 }}>
          <button
            className="settings-button mono"
            disabled={downloading}
            onClick={downloadUpdate}
          >
            {downloading ? `Downloading... ${downloadProgress}%` : `Download v${updateAvailableVersion}`}
          </button>
        </div>
      )}
      {updateDownloadedVersion && (
        <div className="settings-row" style={{ marginTop: 8 }}>
          <button className="settings-button mono" onClick={installUpdate}>
            Restart &amp; Install v{updateDownloadedVersion}
          </button>
        </div>
      )}
      {updateError && (
        <div className="settings-note mono" style={{ color: "#ef4444" }}>
          Update error: {updateError}
        </div>
      )}

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
      <div className="settings-row">
        <label className="settings-label mono">Upload endpoint</label>
        <input
          className="settings-select mono"
          type="text"
          placeholder="https://your-beta-server.example.com/crashes"
          disabled={saving}
          defaultValue={settings.crash_reporting.upload_endpoint ?? ""}
          key={settings.crash_reporting.upload_endpoint ?? ""}
          onBlur={(e) => {
            const trimmed = e.target.value.trim();
            save({ crash_reporting: { upload_endpoint: trimmed.length > 0 ? trimmed : null } });
          }}
          style={{ width: 320 }}
        />
      </div>
      <div className="settings-note mono">
        No default or built-in endpoint of any kind — a report is only ever sent if you type an
        endpoint here yourself and then click "Upload" on a specific report below. Nothing is
        sent automatically, ever, regardless of whether an endpoint is configured.
      </div>
      <div className="settings-row" style={{ alignItems: "flex-start" }}>
        <label className="settings-label mono">Local reports</label>
        <div style={{ display: "flex", flexDirection: "column", gap: 6, flex: 1 }}>
          {crashReportsError && <span className="leo-error mono">{crashReportsError}</span>}
          {crashReports.length === 0 && !crashReportsError && (
            <span className="settings-update-status mono">No crash reports on this machine.</span>
          )}
          {crashReports.map((entry) => {
            const status = uploadStatusLine(uploadStatus[entry.filename]);
            return (
              <div key={entry.filename} className="settings-row" style={{ marginBottom: 0 }}>
                <span className="settings-update-status mono" style={{ flex: 1 }}>
                  {entry.filename} — {entry.report.message.slice(0, 60)}
                </span>
                <button
                  className="settings-button mono"
                  disabled={
                    !settings.crash_reporting.upload_endpoint ||
                    uploadStatus[entry.filename]?.kind === "uploading"
                  }
                  onClick={() => uploadReport(entry.filename)}
                >
                  Upload
                </button>
                {status && <span className="settings-update-status mono">{status}</span>}
              </div>
            );
          })}
          <button className="settings-button mono" onClick={refreshCrashReports}>
            Refresh
          </button>
        </div>
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
