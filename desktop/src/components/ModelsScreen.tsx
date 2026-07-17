import React, { useCallback, useEffect, useState } from "react";

interface ModelStatus {
  configured: boolean;
  kind: string;
  [key: string]: unknown;
}

interface LiteLlmStatus {
  status: "running" | "not_running";
  port?: number;
  pid?: number;
}

interface HfModel {
  id: string;
  display_name: string;
  hf_repo: string;
  tag: string;
  description: string;
}

type PullState =
  | { phase: "idle" }
  | { phase: "pulling"; lines: string[] }
  | { phase: "ready" }
  | { phase: "failed"; error: string };

interface DownloadedGgufModel {
  filename: string;
  size_bytes: number;
  path: string;
}

/** The llama.cpp sibling of `PullState` -- a real `ready` phase carries the
 * real, on-disk `path` the download finished at, since that's exactly what
 * a "Use this model" button needs to hand `settings_set`. Byte-identical to
 * `web/`'s own copy. */
type LlamaCppPullState =
  | { phase: "idle" }
  | { phase: "downloading"; lines: string[] }
  | { phase: "ready"; path: string }
  | { phase: "failed"; error: string };

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${bytes} B`;
}

/**
 * Client-side mirror of `hf_downloader::normalize_hf_repo_input` (Rust) --
 * strips the same real, common pasted-link prefixes down to a bare
 * `<org>/<name>` repo id. Kept in sync deliberately (a small pure string
 * helper, no shared build step between Rust and TypeScript here) so a
 * locally-computed pull-state key matches the real `event_id` the backend
 * computes for the identical input. Byte-identical to `web/`'s own copy in
 * `ModelsPanel.tsx` -- both ports of the same real Rust logic.
 */
function normalizeHfRepoInput(input: string): string {
  const s = input.trim();
  const prefixes = ["https://huggingface.co/", "http://huggingface.co/", "huggingface.co/", "hf.co/"];
  for (const prefix of prefixes) {
    if (s.startsWith(prefix)) {
      return s.slice(prefix.length).replace(/\/+$/, "");
    }
  }
  return s.replace(/\/+$/, "");
}

/**
 * Real Model Management screen for `desktop/` -- the direct Electron
 * sibling of `web/`'s `ModelsPanel.tsx` (task #145: "these features need to
 * be added to the desktop IDE as well"). Before this pass, `model_status`/
 * `litellm_proxy_*`/`hf_*`/`lmstudio_*` only existed on `spartan-devserver`'s
 * own wrapping dispatcher, which `desktop/`'s Electron main process never
 * connects to (it spawns a plain `spartan-backend`) -- closed by moving all
 * of that real logic down into `spartan-backend` itself (see
 * `crates/spartan-backend/src/hf_downloader.rs`/`litellm_proxy.rs`/
 * `lmstudio_downloader.rs`), so the identical real IPC methods `web/` has
 * are now reachable here too via `window.spartan.call`, no `BackendClient`
 * needed. Uses the exact same real backend methods/event names as `web/`'s
 * panel; only the transport (`window.spartan` vs. a `BackendClient`
 * instance) differs.
 */
export default function ModelsScreen(): React.ReactElement {
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [modelStatusError, setModelStatusError] = useState<string | null>(null);

  const [litellmStatus, setLitellmStatus] = useState<LiteLlmStatus | null>(null);
  const [litellmPort, setLitellmPort] = useState("4000");
  const [litellmBusy, setLitellmBusy] = useState(false);
  const [litellmLog, setLitellmLog] = useState<string[]>([]);
  const [litellmError, setLitellmError] = useState<string | null>(null);

  const [hfModels, setHfModels] = useState<HfModel[]>([]);
  const [hfError, setHfError] = useState<string | null>(null);
  const [pullStates, setPullStates] = useState<Record<string, PullState>>({});

  const [customRepo, setCustomRepo] = useState("");
  const [customTag, setCustomTag] = useState("Q4_K_M");
  const [customFormError, setCustomFormError] = useState<string | null>(null);

  // LM Studio's own pull states are kept in a *separate* map from Ollama's
  // above -- both backends' real event-id shape is deliberately identical
  // (`<repo>:<tag>` for a custom pull, the curated `model.id` otherwise),
  // so sharing one map would let an Ollama pull and an LM Studio pull of
  // the exact same curated model silently clobber each other's status.
  const [lmModels, setLmModels] = useState<HfModel[]>([]);
  const [lmError, setLmError] = useState<string | null>(null);
  const [lmAvailable, setLmAvailable] = useState<boolean | null>(null);
  const [lmPullStates, setLmPullStates] = useState<Record<string, PullState>>({});

  const [customLmRepo, setCustomLmRepo] = useState("");
  const [customLmTag, setCustomLmTag] = useState("Q4_K_M");
  const [customLmFormError, setCustomLmFormError] = useState<string | null>(null);

  // llama.cpp's own real, direct HTTP GGUF downloader (task #143) -- see
  // `web/`'s `ModelsPanel.tsx` for the full account of why this is kept
  // separate from the two subprocess-driven maps above.
  const [llamacppModels, setLlamacppModels] = useState<HfModel[]>([]);
  const [llamacppDownloaded, setLlamacppDownloaded] = useState<DownloadedGgufModel[]>([]);
  const [llamacppError, setLlamacppError] = useState<string | null>(null);
  const [llamacppPullStates, setLlamacppPullStates] = useState<Record<string, LlamaCppPullState>>(
    {}
  );
  const [useModelStatus, setUseModelStatus] = useState<string | null>(null);

  const [customLlamacppRepo, setCustomLlamacppRepo] = useState("");
  const [customLlamacppTag, setCustomLlamacppTag] = useState("Q4_K_M");
  const [customLlamacppFormError, setCustomLlamacppFormError] = useState<string | null>(null);

  const refreshModelStatus = useCallback(() => {
    window.spartan
      .call("model_status")
      .then((result) => {
        setModelStatus(result as ModelStatus);
        setModelStatusError(null);
      })
      .catch((e: Error) => setModelStatusError(e.message));
  }, []);

  const refreshLitellmStatus = useCallback(() => {
    window.spartan
      .call("litellm_proxy_status")
      .then((result) => setLitellmStatus(result as LiteLlmStatus))
      .catch((e: Error) => setLitellmError(e.message));
  }, []);

  const refreshHfModels = useCallback(() => {
    window.spartan
      .call("hf_list_models")
      .then((result) => {
        setHfModels((result as { models: HfModel[] }).models);
        setHfError(null);
      })
      .catch((e: Error) => setHfError(e.message));
  }, []);

  const refreshLmStudioModels = useCallback(() => {
    window.spartan
      .call("lmstudio_list_models")
      .then((result) => {
        const r = result as { models: HfModel[]; lms_available: boolean };
        setLmModels(r.models);
        setLmAvailable(r.lms_available);
        setLmError(null);
      })
      .catch((e: Error) => setLmError(e.message));
  }, []);

  const refreshLlamacppModels = useCallback(() => {
    window.spartan
      .call("llamacpp_list_models")
      .then((result) => {
        const r = result as { models: HfModel[]; downloaded: DownloadedGgufModel[] };
        setLlamacppModels(r.models);
        setLlamacppDownloaded(r.downloaded);
        setLlamacppError(null);
      })
      .catch((e: Error) => setLlamacppError(e.message));
  }, []);

  useEffect(() => {
    refreshModelStatus();
    refreshLitellmStatus();
    refreshHfModels();
    refreshLmStudioModels();
    refreshLlamacppModels();
  }, [
    refreshModelStatus,
    refreshLitellmStatus,
    refreshHfModels,
    refreshLmStudioModels,
    refreshLlamacppModels,
  ]);

  useEffect(() => {
    return window.spartan.onEvent((event, data) => {
      if (event === "litellm_progress") {
        const { line } = data as { line: string };
        setLitellmLog((prev) => [...prev.slice(-49), line]);
      } else if (event === "litellm_ready") {
        setLitellmBusy(false);
        setLitellmError(null);
        refreshLitellmStatus();
      } else if (event === "litellm_failed") {
        const { error } = data as { error: string };
        setLitellmBusy(false);
        setLitellmError(error);
        refreshLitellmStatus();
      } else if (event === "hf_pull_progress") {
        const { model_id, line } = data as { model_id: string; line: string };
        setPullStates((prev) => {
          const existing = prev[model_id];
          const lines = existing?.phase === "pulling" ? existing.lines : [];
          return { ...prev, [model_id]: { phase: "pulling", lines: [...lines.slice(-49), line] } };
        });
      } else if (event === "hf_pull_ready") {
        const { model_id } = data as { model_id: string };
        setPullStates((prev) => ({ ...prev, [model_id]: { phase: "ready" } }));
      } else if (event === "hf_pull_failed") {
        const { model_id, error } = data as { model_id: string; error: string };
        setPullStates((prev) => ({ ...prev, [model_id]: { phase: "failed", error } }));
      } else if (event === "lmstudio_pull_progress") {
        const { model_id, line } = data as { model_id: string; line: string };
        setLmPullStates((prev) => {
          const existing = prev[model_id];
          const lines = existing?.phase === "pulling" ? existing.lines : [];
          return { ...prev, [model_id]: { phase: "pulling", lines: [...lines.slice(-49), line] } };
        });
      } else if (event === "lmstudio_pull_ready") {
        const { model_id } = data as { model_id: string };
        setLmPullStates((prev) => ({ ...prev, [model_id]: { phase: "ready" } }));
      } else if (event === "lmstudio_pull_failed") {
        const { model_id, error } = data as { model_id: string; error: string };
        setLmPullStates((prev) => ({ ...prev, [model_id]: { phase: "failed", error } }));
      } else if (event === "llamacpp_download_progress") {
        const { model_id, line } = data as { model_id: string; line: string };
        setLlamacppPullStates((prev) => {
          const existing = prev[model_id];
          const lines = existing?.phase === "downloading" ? existing.lines : [];
          return {
            ...prev,
            [model_id]: { phase: "downloading", lines: [...lines.slice(-49), line] },
          };
        });
      } else if (event === "llamacpp_download_ready") {
        const { model_id, path } = data as { model_id: string; path: string };
        setLlamacppPullStates((prev) => ({ ...prev, [model_id]: { phase: "ready", path } }));
        refreshLlamacppModels();
      } else if (event === "llamacpp_download_failed") {
        const { model_id, error } = data as { model_id: string; error: string };
        setLlamacppPullStates((prev) => ({ ...prev, [model_id]: { phase: "failed", error } }));
      }
    });
  }, [refreshLitellmStatus, refreshLlamacppModels]);

  const startLitellm = useCallback(() => {
    const port = Number.parseInt(litellmPort, 10);
    if (!Number.isFinite(port) || port <= 0) {
      setLitellmError("enter a valid port number");
      return;
    }
    setLitellmBusy(true);
    setLitellmError(null);
    setLitellmLog([]);
    window.spartan.call("litellm_proxy_start", { port }).catch((e: Error) => {
      setLitellmBusy(false);
      setLitellmError(e.message);
    });
  }, [litellmPort]);

  const stopLitellm = useCallback(() => {
    setLitellmBusy(true);
    window.spartan
      .call("litellm_proxy_stop")
      .then(() => {
        setLitellmBusy(false);
        refreshLitellmStatus();
      })
      .catch((e: Error) => {
        setLitellmBusy(false);
        setLitellmError(e.message);
      });
  }, [refreshLitellmStatus]);

  const pullModel = useCallback((modelId: string) => {
    setPullStates((prev) => ({ ...prev, [modelId]: { phase: "pulling", lines: [] } }));
    window.spartan.call("hf_pull_model", { model_id: modelId }).catch((e: Error) => {
      setPullStates((prev) => ({ ...prev, [modelId]: { phase: "failed", error: e.message } }));
    });
  }, []);

  const pullCustomModel = useCallback(() => {
    const repo = customRepo.trim();
    const tag = customTag.trim();
    if (!repo || !tag) {
      setCustomFormError("enter both a repo (org/name or a pasted HF link) and a quant tag");
      return;
    }
    const key = `${normalizeHfRepoInput(repo)}:${tag}`;
    setCustomFormError(null);
    setPullStates((prev) => ({ ...prev, [key]: { phase: "pulling", lines: [] } }));
    window.spartan.call("hf_pull_model", { hf_repo: repo, tag }).catch((e: Error) => {
      setPullStates((prev) => ({ ...prev, [key]: { phase: "failed", error: e.message } }));
    });
  }, [customRepo, customTag]);

  const pullLmModel = useCallback((modelId: string) => {
    setLmPullStates((prev) => ({ ...prev, [modelId]: { phase: "pulling", lines: [] } }));
    window.spartan.call("lmstudio_pull_model", { model_id: modelId }).catch((e: Error) => {
      setLmPullStates((prev) => ({ ...prev, [modelId]: { phase: "failed", error: e.message } }));
    });
  }, []);

  const pullCustomLmModel = useCallback(() => {
    const repo = customLmRepo.trim();
    const tag = customLmTag.trim();
    if (!repo || !tag) {
      setCustomLmFormError("enter both a repo (org/name or a pasted HF link) and a quant tag");
      return;
    }
    const key = `${normalizeHfRepoInput(repo)}:${tag}`;
    setCustomLmFormError(null);
    setLmPullStates((prev) => ({ ...prev, [key]: { phase: "pulling", lines: [] } }));
    window.spartan.call("lmstudio_pull_model", { hf_repo: repo, tag }).catch((e: Error) => {
      setLmPullStates((prev) => ({ ...prev, [key]: { phase: "failed", error: e.message } }));
    });
  }, [customLmRepo, customLmTag]);

  /** Real, direct HTTP download for a curated llama.cpp model. */
  const downloadLlamacppModel = useCallback((modelId: string) => {
    setLlamacppPullStates((prev) => ({ ...prev, [modelId]: { phase: "downloading", lines: [] } }));
    window.spartan.call("llamacpp_download_model", { model_id: modelId }).catch((e: Error) => {
      setLlamacppPullStates((prev) => ({
        ...prev,
        [modelId]: { phase: "failed", error: e.message },
      }));
    });
  }, []);

  /** The llama.cpp sibling of `pullCustomModel`/`pullCustomLmModel` -- any
   * real, public HF GGUF repo, downloaded directly (no local server). */
  const downloadCustomLlamacppModel = useCallback(() => {
    const repo = customLlamacppRepo.trim();
    const tag = customLlamacppTag.trim();
    if (!repo || !tag) {
      setCustomLlamacppFormError("enter both a repo (org/name or a pasted HF link) and a quant tag");
      return;
    }
    const key = `${normalizeHfRepoInput(repo)}:${tag}`;
    setCustomLlamacppFormError(null);
    setLlamacppPullStates((prev) => ({ ...prev, [key]: { phase: "downloading", lines: [] } }));
    window.spartan.call("llamacpp_download_model", { hf_repo: repo, tag }).catch((e: Error) => {
      setLlamacppPullStates((prev) => ({ ...prev, [key]: { phase: "failed", error: e.message } }));
    });
  }, [customLlamacppRepo, customLlamacppTag]);

  /**
   * Sets a real, already-downloaded `.gguf` file as Leo's active local
   * provider. Fetches the real current settings first -- `settings_set`'s
   * `gpu_enabled` param is mandatory (no fallback), so this can't just send
   * `leo_provider` alone without first reading what's already saved.
   */
  const useAsLlamaCppProvider = useCallback((path: string) => {
    setUseModelStatus(`Setting ${path} as the active model…`);
    window.spartan
      .call("settings_get")
      .then((current) => {
        const s = current as { gpu_offload?: { enabled: boolean; layers?: number } };
        return window.spartan.call("settings_set", {
          gpu_enabled: s.gpu_offload?.enabled ?? false,
          gpu_layers: s.gpu_offload?.layers,
          leo_provider: { kind: "LlamaCpp", model: path },
        });
      })
      .then(() => setUseModelStatus(`✓ ${path} set as Leo's active local model.`))
      .catch((e: Error) => setUseModelStatus(`Failed: ${e.message}`));
  }, []);

  return (
    <div className="git-panel">
      <div className="git-section-label mono">Local Model Provider</div>
      <div className="git-section">
        {modelStatusError && <div className="git-panel-empty mono">{modelStatusError}</div>}
        {!modelStatusError && !modelStatus && (
          <div className="git-panel-empty mono">Loading…</div>
        )}
        {!modelStatusError && modelStatus && (
          <div className="git-row" style={{ cursor: "default" }}>
            <span className="git-status-glyph mono">{modelStatus.configured ? "●" : "○"}</span>
            <span className="mono git-row-path">
              {modelStatus.kind}
              {modelStatus.configured ? " — configured" : " — not configured"}
            </span>
          </div>
        )}
      </div>

      <div className="git-section-label mono">LiteLLM Proxy</div>
      <div className="git-section">
        <div className="git-row" style={{ cursor: "default" }}>
          <span className="git-status-glyph mono">
            {litellmStatus?.status === "running" ? "●" : "○"}
          </span>
          <span className="mono git-row-path">
            {litellmStatus?.status === "running"
              ? `running on port ${litellmStatus.port} (pid ${litellmStatus.pid})`
              : "not running"}
          </span>
        </div>
        {litellmError && <div className="git-panel-empty mono">{litellmError}</div>}
        {litellmStatus?.status !== "running" ? (
          <div style={{ display: "flex", gap: 6, padding: "4px 4px" }}>
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, width: 70, resize: "none" }}
              value={litellmPort}
              onChange={(e) => setLitellmPort(e.target.value)}
              placeholder="port"
            />
            <button className="git-commit-button" disabled={litellmBusy} onClick={startLitellm}>
              {litellmBusy ? "Starting…" : "Start"}
            </button>
          </div>
        ) : (
          <div style={{ padding: "4px 4px" }}>
            <button className="git-commit-button" disabled={litellmBusy} onClick={stopLitellm}>
              {litellmBusy ? "Stopping…" : "Stop"}
            </button>
          </div>
        )}
        {litellmLog.length > 0 && (
          <pre className="mono" style={{ fontSize: 10.5, maxHeight: 120, overflowY: "auto", margin: "4px" }}>
            {litellmLog.join("\n")}
          </pre>
        )}
      </div>

      <div className="git-section-label mono">Hugging Face Models (via Ollama)</div>
      <div className="git-section">
        {hfError && <div className="git-panel-empty mono">{hfError}</div>}
        {hfModels.map((model) => {
          const state = pullStates[model.id] ?? { phase: "idle" as const };
          return (
            <div key={model.id} style={{ padding: "4px 4px", borderBottom: "1px solid var(--border)" }}>
              <div className="mono" style={{ fontSize: 12 }}>
                {model.display_name}
              </div>
              <div className="mono" style={{ fontSize: 10.5, color: "var(--text-dim)" }}>
                {model.description}
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 3 }}>
                <button
                  className="git-commit-button"
                  disabled={state.phase === "pulling"}
                  onClick={() => pullModel(model.id)}
                >
                  {state.phase === "pulling"
                    ? "Pulling…"
                    : state.phase === "ready"
                      ? "Pull again"
                      : "Pull"}
                </button>
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "#e05a4a" }}>
                    {state.error}
                  </span>
                )}
              </div>
              {state.phase === "pulling" && state.lines.length > 0 && (
                <pre
                  className="mono"
                  style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0" }}
                >
                  {state.lines.join("\n")}
                </pre>
              )}
            </div>
          );
        })}
      </div>

      <div className="git-section-label mono">Custom Model Link (any public HF GGUF repo)</div>
      <div className="git-section">
        <div style={{ padding: "4px 4px" }}>
          <div className="mono" style={{ fontSize: 10.5, color: "var(--text-dim)", marginBottom: 4 }}>
            Paste a Hugging Face repo (e.g. <code>org/name-GGUF</code> or a full
            huggingface.co/hf.co link) and the exact quant tag from its file list.
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, flex: 2, minWidth: 180, resize: "none" }}
              value={customRepo}
              onChange={(e) => setCustomRepo(e.target.value)}
              placeholder="org/name-GGUF or https://huggingface.co/org/name-GGUF"
            />
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, flex: 1, minWidth: 80, resize: "none" }}
              value={customTag}
              onChange={(e) => setCustomTag(e.target.value)}
              placeholder="Q4_K_M"
            />
            <button
              className="git-commit-button"
              disabled={pullStates[`${normalizeHfRepoInput(customRepo)}:${customTag.trim()}`]?.phase === "pulling"}
              onClick={pullCustomModel}
            >
              {pullStates[`${normalizeHfRepoInput(customRepo)}:${customTag.trim()}`]?.phase === "pulling"
                ? "Pulling…"
                : "Pull"}
            </button>
          </div>
          {customFormError && (
            <div className="git-panel-empty mono" style={{ marginTop: 4 }}>
              {customFormError}
            </div>
          )}
          {(() => {
            const key = `${normalizeHfRepoInput(customRepo)}:${customTag.trim()}`;
            const state = pullStates[key];
            if (!state || state.phase === "idle") return null;
            return (
              <div style={{ marginTop: 4 }}>
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "#e05a4a" }}>
                    {state.error}
                  </span>
                )}
                {state.phase === "pulling" && state.lines.length > 0 && (
                  <pre
                    className="mono"
                    style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0" }}
                  >
                    {state.lines.join("\n")}
                  </pre>
                )}
              </div>
            );
          })()}
        </div>
      </div>

      <div className="git-section-label mono">LM Studio Models</div>
      <div className="git-section">
        {lmError && <div className="git-panel-empty mono">{lmError}</div>}
        {lmAvailable !== null && (
          <div
            className="mono"
            style={{
              fontSize: 10.5,
              padding: "4px 4px",
              color: lmAvailable ? "var(--accent)" : "var(--text-dim)",
            }}
          >
            {lmAvailable
              ? "✓ LM Studio detected — pulls run through its bundled lms CLI, no setup needed."
              : "LM Studio not detected. Install it from lmstudio.ai and open it once — lms " +
                "ships bundled, no PATH setup required, then Pull below will work."}
          </div>
        )}
        {lmModels.map((model) => {
          const state = lmPullStates[model.id] ?? { phase: "idle" as const };
          return (
            <div
              key={model.id}
              style={{ padding: "4px 4px", borderBottom: "1px solid var(--border)" }}
            >
              <div className="mono" style={{ fontSize: 12 }}>
                {model.display_name}
              </div>
              <div className="mono" style={{ fontSize: 10.5, color: "var(--text-dim)" }}>
                {model.description}
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 3 }}>
                <button
                  className="git-commit-button"
                  disabled={state.phase === "pulling"}
                  onClick={() => pullLmModel(model.id)}
                >
                  {state.phase === "pulling"
                    ? "Pulling…"
                    : state.phase === "ready"
                      ? "Pull again"
                      : "Pull"}
                </button>
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "#e05a4a" }}>
                    {state.error}
                  </span>
                )}
              </div>
              {state.phase === "pulling" && state.lines.length > 0 && (
                <pre
                  className="mono"
                  style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0" }}
                >
                  {state.lines.join("\n")}
                </pre>
              )}
            </div>
          );
        })}
      </div>

      <div className="git-section-label mono">
        Custom LM Studio Model Link (any public HF GGUF repo)
      </div>
      <div className="git-section">
        <div style={{ padding: "4px 4px" }}>
          <div
            className="mono"
            style={{ fontSize: 10.5, color: "var(--text-dim)", marginBottom: 4 }}
          >
            Same repo/tag format as above -- pulled through LM Studio's own <code>lms</code>{" "}
            CLI instead of Ollama.
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, flex: 2, minWidth: 180, resize: "none" }}
              value={customLmRepo}
              onChange={(e) => setCustomLmRepo(e.target.value)}
              placeholder="org/name-GGUF or https://huggingface.co/org/name-GGUF"
            />
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, flex: 1, minWidth: 80, resize: "none" }}
              value={customLmTag}
              onChange={(e) => setCustomLmTag(e.target.value)}
              placeholder="Q4_K_M"
            />
            <button
              className="git-commit-button"
              disabled={
                lmPullStates[`${normalizeHfRepoInput(customLmRepo)}:${customLmTag.trim()}`]
                  ?.phase === "pulling"
              }
              onClick={pullCustomLmModel}
            >
              {lmPullStates[`${normalizeHfRepoInput(customLmRepo)}:${customLmTag.trim()}`]
                ?.phase === "pulling"
                ? "Pulling…"
                : "Pull"}
            </button>
          </div>
          {customLmFormError && (
            <div className="git-panel-empty mono" style={{ marginTop: 4 }}>
              {customLmFormError}
            </div>
          )}
          {(() => {
            const key = `${normalizeHfRepoInput(customLmRepo)}:${customLmTag.trim()}`;
            const state = lmPullStates[key];
            if (!state || state.phase === "idle") return null;
            return (
              <div style={{ marginTop: 4 }}>
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "#e05a4a" }}>
                    {state.error}
                  </span>
                )}
                {state.phase === "pulling" && state.lines.length > 0 && (
                  <pre
                    className="mono"
                    style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0" }}
                  >
                    {state.lines.join("\n")}
                  </pre>
                )}
              </div>
            );
          })()}
        </div>
      </div>

      <div className="git-section-label mono">llama.cpp Models (direct local download)</div>
      <div className="git-section">
        <div className="mono" style={{ fontSize: 10.5, padding: "4px 4px", color: "var(--text-dim)" }}>
          Unlike Ollama/LM Studio, llama.cpp runs in-process -- there's no local server to hand a
          pull request to, so this downloads the real .gguf file directly into ~/.spartan/models.
        </div>
        {llamacppError && <div className="git-panel-empty mono">{llamacppError}</div>}
        {useModelStatus && (
          <div className="mono" style={{ fontSize: 10.5, padding: "4px 4px", color: "var(--accent)" }}>
            {useModelStatus}
          </div>
        )}
        {llamacppModels.map((model) => {
          const state = llamacppPullStates[model.id] ?? { phase: "idle" as const };
          return (
            <div key={model.id} style={{ padding: "4px 4px", borderBottom: "1px solid var(--border)" }}>
              <div className="mono" style={{ fontSize: 12 }}>
                {model.display_name}
              </div>
              <div className="mono" style={{ fontSize: 10.5, color: "var(--text-dim)" }}>
                {model.description}
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 3, flexWrap: "wrap" }}>
                <button
                  className="git-commit-button"
                  disabled={state.phase === "downloading"}
                  onClick={() => downloadLlamacppModel(model.id)}
                >
                  {state.phase === "downloading"
                    ? "Downloading…"
                    : state.phase === "ready"
                      ? "Download again"
                      : "Download"}
                </button>
                {state.phase === "ready" && (
                  <>
                    <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                      ✓ ready
                    </span>
                    <button className="git-commit-button" onClick={() => useAsLlamaCppProvider(state.path)}>
                      Use this model
                    </button>
                  </>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "#e05a4a" }}>
                    {state.error}
                  </span>
                )}
              </div>
              {state.phase === "downloading" && state.lines.length > 0 && (
                <pre className="mono" style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0" }}>
                  {state.lines.join("\n")}
                </pre>
              )}
            </div>
          );
        })}
      </div>

      <div className="git-section-label mono">
        Custom llama.cpp Model Link (any public HF GGUF repo)
      </div>
      <div className="git-section">
        <div style={{ padding: "4px 4px" }}>
          <div className="mono" style={{ fontSize: 10.5, color: "var(--text-dim)", marginBottom: 4 }}>
            Same repo/tag format as above -- downloaded directly, no local server needed.
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, flex: 2, minWidth: 180, resize: "none" }}
              value={customLlamacppRepo}
              onChange={(e) => setCustomLlamacppRepo(e.target.value)}
              placeholder="org/name-GGUF or https://huggingface.co/org/name-GGUF"
            />
            <input
              className="git-commit-input mono"
              style={{ minHeight: 0, flex: 1, minWidth: 80, resize: "none" }}
              value={customLlamacppTag}
              onChange={(e) => setCustomLlamacppTag(e.target.value)}
              placeholder="Q4_K_M"
            />
            <button
              className="git-commit-button"
              disabled={
                llamacppPullStates[`${normalizeHfRepoInput(customLlamacppRepo)}:${customLlamacppTag.trim()}`]
                  ?.phase === "downloading"
              }
              onClick={downloadCustomLlamacppModel}
            >
              {llamacppPullStates[`${normalizeHfRepoInput(customLlamacppRepo)}:${customLlamacppTag.trim()}`]
                ?.phase === "downloading"
                ? "Downloading…"
                : "Download"}
            </button>
          </div>
          {customLlamacppFormError && (
            <div className="git-panel-empty mono" style={{ marginTop: 4 }}>
              {customLlamacppFormError}
            </div>
          )}
          {(() => {
            const key = `${normalizeHfRepoInput(customLlamacppRepo)}:${customLlamacppTag.trim()}`;
            const state = llamacppPullStates[key];
            if (!state || state.phase === "idle") return null;
            return (
              <div style={{ marginTop: 4, display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                {state.phase === "ready" && (
                  <>
                    <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                      ✓ ready
                    </span>
                    <button className="git-commit-button" onClick={() => useAsLlamaCppProvider(state.path)}>
                      Use this model
                    </button>
                  </>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "#e05a4a" }}>
                    {state.error}
                  </span>
                )}
                {state.phase === "downloading" && state.lines.length > 0 && (
                  <pre className="mono" style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0" }}>
                    {state.lines.join("\n")}
                  </pre>
                )}
              </div>
            );
          })()}
        </div>
      </div>

      <div className="git-section-label mono">Downloaded GGUF Files</div>
      <div className="git-section">
        {llamacppDownloaded.length === 0 && (
          <div className="git-panel-empty mono">No .gguf files downloaded yet.</div>
        )}
        {llamacppDownloaded.map((d) => (
          <div key={d.filename} className="git-row" style={{ cursor: "default" }}>
            <span className="git-status-glyph mono">●</span>
            <span className="mono git-row-path" style={{ flex: 1 }}>
              {d.filename} ({formatBytes(d.size_bytes)})
            </span>
            <button className="git-commit-button" onClick={() => useAsLlamaCppProvider(d.path)}>
              Use this model
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
