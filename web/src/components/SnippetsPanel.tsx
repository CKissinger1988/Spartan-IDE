import React, { useCallback, useEffect, useState } from "react";
import type { BackendClient } from "../backendClient";
import type { UserSnippet } from "../snippets";

interface SnippetsPanelProps {
  client: BackendClient;
}

/**
 * Real user-defined snippet manager for the web shell (the "Snippets"
 * sidebar view, available whenever a devserver is connected) -- the direct
 * counterpart of `desktop/`'s User Snippets settings section. Snippets are
 * stored in the exact same `Settings.user_snippets` list (`settings_get`/
 * `settings_set`) the desktop app persists to, so a web session on the same
 * devserver machine sees the snippets its desktop app created and vice
 * versa. This panel has no `gpu_enabled`-style full-settings editor, so it
 * follows `ModelsPanel.tsx`'s own already-established partial-update
 * pattern: read the current settings first (the mandatory `gpu_enabled`
 * param has no fallback), then send `settings_set` with the full edited
 * snippet list plus the untouched GPU values.
 */
export default function SnippetsPanel({ client }: SnippetsPanelProps): React.ReactElement {
  const [snippets, setSnippets] = useState<UserSnippet[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    client
      .call("settings_get")
      .then((result) => {
        if (cancelled) return;
        const s = result as { user_snippets?: UserSnippet[] };
        setSnippets(s.user_snippets ?? []);
        setLoadError(null);
      })
      .catch((e: Error) => {
        if (cancelled) return;
        setLoadError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [client]);

  const setField = useCallback((index: number, field: keyof UserSnippet, value: string) => {
    setError(null);
    setStatus(null);
    setSnippets((prev) =>
      prev ? prev.map((s, i) => (i === index ? { ...s, [field]: value } : s)) : prev
    );
  }, []);

  const removeSnippet = useCallback((index: number) => {
    setError(null);
    setStatus(null);
    setSnippets((prev) => (prev ? prev.filter((_, i) => i !== index) : prev));
  }, []);

  const addSnippet = useCallback(() => {
    setError(null);
    setStatus(null);
    setSnippets((prev) => [
      ...(prev ?? []),
      { lang_id: "", prefix: "", body: "", description: "" },
    ]);
  }, []);

  const save = useCallback(() => {
    if (!snippets) return;
    // Real, honest client-side validation mirroring `spartan-backend`'s
    // own `settings_set` rules exactly (non-empty `lang_id`/`prefix`/`body`)
    // -- catch it in the UI instead of letting the backend fail the whole
    // save with a less friendly error.
    const trimmed = snippets.map((s) => ({
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
      setError(`Snippet ${bad + 1}: ${field} must not be empty`);
      return;
    }
    setSaving(true);
    setStatus(null);
    setError(null);
    client
      .call("settings_get")
      .then((current) => {
        const s = current as { gpu_offload?: { enabled: boolean; layers?: number } };
        return client.call("settings_set", {
          gpu_enabled: s.gpu_offload?.enabled ?? false,
          gpu_layers: s.gpu_offload?.layers,
          user_snippets: trimmed,
        });
      })
      .then(() => {
        setStatus("✓ Saved");
        setSnippets(trimmed);
      })
      .catch((e: Error) => setError(`Save failed: ${e.message}`))
      .finally(() => setSaving(false));
  }, [client, snippets]);

  return (
    <div className="git-panel">
      <div className="git-section-label mono">User Snippets</div>
      <div className="git-section">
        {loadError && <div className="git-panel-empty mono">{loadError}</div>}
        {!loadError && snippets === null && <div className="git-panel-empty mono">Loading…</div>}
        {!loadError &&
          snippets !== null &&
          snippets.map((snip, i) => (
            <div key={i} style={{ display: "flex", flexDirection: "column", gap: 6, margin: "6px 0" }}>
              <div className="git-row" style={{ cursor: "default", whiteSpace: "normal" }}>
                <span className="settings-label mono" style={{ width: 64, flexShrink: 0 }}>
                  Language
                </span>
                <input
                  className="settings-select mono"
                  type="text"
                  value={snip.lang_id}
                  disabled={saving}
                  onChange={(e) => setField(i, "lang_id", e.target.value)}
                  style={{ width: 110 }}
                />
                <span className="settings-label mono" style={{ width: 44, flexShrink: 0 }}>
                  Prefix
                </span>
                <input
                  className="settings-select mono"
                  type="text"
                  value={snip.prefix}
                  disabled={saving}
                  onChange={(e) => setField(i, "prefix", e.target.value)}
                  style={{ width: 100 }}
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
              <div className="git-row" style={{ cursor: "default", whiteSpace: "normal", alignItems: "flex-start" }}>
                <span className="settings-label mono" style={{ width: 64, flexShrink: 0 }}>
                  Body
                </span>
                <textarea
                  className="settings-select mono"
                  value={snip.body}
                  disabled={saving}
                  onChange={(e) => setField(i, "body", e.target.value)}
                  rows={3}
                  spellCheck={false}
                  style={{ width: 280, fontFamily: "var(--font-mono)", resize: "vertical" }}
                />
              </div>
              <div className="git-row" style={{ cursor: "default", whiteSpace: "normal" }}>
                <span className="settings-label mono" style={{ width: 64, flexShrink: 0 }}>
                  Description
                </span>
                <input
                  className="settings-select mono"
                  type="text"
                  value={snip.description}
                  disabled={saving}
                  onChange={(e) => setField(i, "description", e.target.value)}
                  style={{ width: 280 }}
                />
              </div>
            </div>
          ))}
        {!loadError && snippets !== null && snippets.length === 0 && (
          <div className="git-panel-empty mono">No user snippets defined. Add one to get started.</div>
        )}
        {error && <div className="leo-error mono">{error}</div>}
        {status && <div className="git-panel-empty mono">{status}</div>}
        {!loadError && snippets !== null && (
          <div className="git-row" style={{ cursor: "default" }}>
            <button className="settings-button mono" disabled={saving} onClick={addSnippet}>
              Add snippet
            </button>
            <button className="settings-button mono" disabled={saving} onClick={save}>
              {saving ? "Saving…" : "Save snippets"}
            </button>
          </div>
        )}
        <div className="git-panel-empty mono">
          Type a prefix and press Tab in a file of the matching language to expand it. Language is
          the editor's own id for that file type (python, rust, typescript, go, …). Bodies use the
          same template syntax as the built-in snippets: {"${1:name}"} for a numbered stop, $0 for
          the final cursor position.
        </div>
      </div>
    </div>
  );
}
