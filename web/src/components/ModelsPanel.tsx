import React, { useCallback, useEffect, useState } from "react";
import type { BackendClient } from "../backendClient";

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
  | { phase: "failed"; error: string; cancelled?: boolean };

interface DownloadedGgufModel {
  filename: string;
  size_bytes: number;
  path: string;
}

/** The llama.cpp sibling of `PullState` -- a real `ready` phase carries the
 * real, on-disk `path` the download finished at, since that's exactly what
 * a "Use this model" button needs to hand `settings_set` -- unlike Ollama/
 * LM Studio, there's no separate local server to report readiness some
 * other way. */
type LlamaCppPullState =
  | { phase: "idle" }
  | { phase: "downloading"; lines: string[] }
  | { phase: "ready"; path: string }
  | { phase: "failed"; error: string; cancelled?: boolean };

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${bytes} B`;
}

interface ModelsPanelProps {
  client: BackendClient;
}

/**
 * Client-side mirror of `hf_downloader::normalize_hf_repo_input` (Rust) --
 * strips the same real, common pasted-link prefixes down to a bare
 * `<org>/<name>` repo id. Kept in sync deliberately (not shared code, since
 * this is a small pure string helper and the two languages don't share a
 * build step here) so a locally-computed pull-state key matches the real
 * `event_id` the backend computes for the identical input, letting the UI
 * show pulling/ready/failed state immediately rather than only after the
 * first `hf_pull_progress` event arrives.
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
 * Real Model Management panel -- the first UI surface for the real Track A
 * devserver-only methods (`model_status`, `litellm_proxy_start`/`_stop`/
 * `_status`, `hf_list_models`/`hf_pull_model`, `lmstudio_list_models`/
 * `lmstudio_pull_model`) that have had zero callers anywhere in either
 * shell since they landed (tasks #128, #138, #139, #144). A direct sibling
 * of `GitPanel.tsx`'s own shape -- one `BackendClient`, `.call()`ed
 * directly, `.onEvent()` subscribed for the async litellm/HF/LM Studio
 * progress events, no new protocol needed since `spartan-devserver`'s
 * dispatcher already answers every one of these methods for real.
 *
 * The Ollama (HF) and LM Studio sections share the exact same real,
 * individually-verified curated model list (`hf_list_models`/
 * `lmstudio_list_models` both ultimately read `hf_downloader::
 * CURATED_MODELS`) -- one real source of truth for "top-rated coding
 * models," driven through two different local backends, rather than two
 * lists a user has to reconcile themselves.
 *
 * These methods only exist on `spartan-devserver`'s own wrapping
 * dispatcher (not `spartan-backend`'s), so this panel is real and reachable
 * here in `web/` -- `desktop/` talks to a plain `spartan-backend` process
 * directly and has no equivalent connection, a real, named platform
 * difference, not an oversight.
 */
export default function ModelsPanel({ client }: ModelsPanelProps): React.ReactElement {
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
  // above -- both backends' event_id shape is deliberately identical
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

  // llama.cpp's own real, direct HTTP GGUF downloader (task #143) -- kept
  // entirely separate from the two subprocess-driven maps above, since a
  // real llama.cpp download's "ready" state carries an on-disk `path`
  // neither Ollama's nor LM Studio's own event shape has any equivalent
  // of (their pull targets become usable through their own already-running
  // local server, never a file path this UI hands back to `settings_set`).
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
    client
      .call("model_status")
      .then((result) => {
        setModelStatus(result as ModelStatus);
        setModelStatusError(null);
      })
      .catch((e: Error) => setModelStatusError(e.message));
  }, [client]);

  const refreshLitellmStatus = useCallback(() => {
    client
      .call("litellm_proxy_status")
      .then((result) => setLitellmStatus(result as LiteLlmStatus))
      .catch((e: Error) => setLitellmError(e.message));
  }, [client]);

  const refreshHfModels = useCallback(() => {
    client
      .call("hf_list_models")
      .then((result) => {
        setHfModels((result as { models: HfModel[] }).models);
        setHfError(null);
      })
      .catch((e: Error) => setHfError(e.message));
  }, [client]);

  const refreshLmStudioModels = useCallback(() => {
    client
      .call("lmstudio_list_models")
      .then((result) => {
        const r = result as { models: HfModel[]; lms_available: boolean };
        setLmModels(r.models);
        setLmAvailable(r.lms_available);
        setLmError(null);
      })
      .catch((e: Error) => setLmError(e.message));
  }, [client]);

  const refreshLlamacppModels = useCallback(() => {
    client
      .call("llamacpp_list_models")
      .then((result) => {
        const r = result as { models: HfModel[]; downloaded: DownloadedGgufModel[] };
        setLlamacppModels(r.models);
        setLlamacppDownloaded(r.downloaded);
        setLlamacppError(null);
      })
      .catch((e: Error) => setLlamacppError(e.message));
  }, [client]);

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
    return client.onEvent((e) => {
      if (e.event === "litellm_progress") {
        const { line } = e.data as { line: string };
        setLitellmLog((prev) => [...prev.slice(-49), line]);
      } else if (e.event === "litellm_ready") {
        setLitellmBusy(false);
        setLitellmError(null);
        refreshLitellmStatus();
      } else if (e.event === "litellm_failed") {
        const { error } = e.data as { error: string };
        setLitellmBusy(false);
        setLitellmError(error);
        refreshLitellmStatus();
      } else if (e.event === "hf_pull_progress") {
        const { model_id, line } = e.data as { model_id: string; line: string };
        setPullStates((prev) => {
          const existing = prev[model_id];
          const lines = existing?.phase === "pulling" ? existing.lines : [];
          return { ...prev, [model_id]: { phase: "pulling", lines: [...lines.slice(-49), line] } };
        });
      } else if (e.event === "hf_pull_ready") {
        const { model_id } = e.data as { model_id: string };
        setPullStates((prev) => ({ ...prev, [model_id]: { phase: "ready" } }));
      } else if (e.event === "hf_pull_failed") {
        const { model_id, error, cancelled } = e.data as {
          model_id: string;
          error: string;
          cancelled?: boolean;
        };
        setPullStates((prev) => ({ ...prev, [model_id]: { phase: "failed", error, cancelled } }));
      } else if (e.event === "lmstudio_pull_progress") {
        const { model_id, line } = e.data as { model_id: string; line: string };
        setLmPullStates((prev) => {
          const existing = prev[model_id];
          const lines = existing?.phase === "pulling" ? existing.lines : [];
          return { ...prev, [model_id]: { phase: "pulling", lines: [...lines.slice(-49), line] } };
        });
      } else if (e.event === "lmstudio_pull_ready") {
        const { model_id } = e.data as { model_id: string };
        setLmPullStates((prev) => ({ ...prev, [model_id]: { phase: "ready" } }));
      } else if (e.event === "lmstudio_pull_failed") {
        const { model_id, error, cancelled } = e.data as {
          model_id: string;
          error: string;
          cancelled?: boolean;
        };
        setLmPullStates((prev) => ({ ...prev, [model_id]: { phase: "failed", error, cancelled } }));
      } else if (e.event === "llamacpp_download_progress") {
        const { model_id, line } = e.data as { model_id: string; line: string };
        setLlamacppPullStates((prev) => {
          const existing = prev[model_id];
          const lines = existing?.phase === "downloading" ? existing.lines : [];
          return {
            ...prev,
            [model_id]: { phase: "downloading", lines: [...lines.slice(-49), line] },
          };
        });
      } else if (e.event === "llamacpp_download_ready") {
        const { model_id, path } = e.data as { model_id: string; path: string };
        setLlamacppPullStates((prev) => ({ ...prev, [model_id]: { phase: "ready", path } }));
        refreshLlamacppModels();
      } else if (e.event === "llamacpp_download_failed") {
        const { model_id, error, cancelled } = e.data as {
          model_id: string;
          error: string;
          cancelled?: boolean;
        };
        setLlamacppPullStates((prev) => ({
          ...prev,
          [model_id]: { phase: "failed", error, cancelled },
        }));
      }
    });
  }, [client, refreshLitellmStatus, refreshLlamacppModels]);

  const startLitellm = useCallback(() => {
    const port = Number.parseInt(litellmPort, 10);
    if (!Number.isFinite(port) || port <= 0) {
      setLitellmError("enter a valid port number");
      return;
    }
    setLitellmBusy(true);
    setLitellmError(null);
    setLitellmLog([]);
    client.call("litellm_proxy_start", { port }).catch((e: Error) => {
      setLitellmBusy(false);
      setLitellmError(e.message);
    });
  }, [client, litellmPort]);

  const stopLitellm = useCallback(() => {
    setLitellmBusy(true);
    client
      .call("litellm_proxy_stop")
      .then(() => {
        setLitellmBusy(false);
        refreshLitellmStatus();
      })
      .catch((e: Error) => {
        setLitellmBusy(false);
        setLitellmError(e.message);
      });
  }, [client, refreshLitellmStatus]);

  const pullModel = useCallback(
    (modelId: string) => {
      setPullStates((prev) => ({ ...prev, [modelId]: { phase: "pulling", lines: [] } }));
      client.call("hf_pull_model", { model_id: modelId }).catch((e: Error) => {
        setPullStates((prev) => ({ ...prev, [modelId]: { phase: "failed", error: e.message } }));
      });
    },
    [client]
  );

  /**
   * The real "user defined model download links" path -- any real, public,
   * anonymously-pullable HF GGUF repo, not just a curated entry. Goes
   * through the identical `hf_pull_model` backend method and identical
   * `hf_pull_progress`/`hf_pull_ready`/`hf_pull_failed` event plumbing as a
   * curated pull; the only difference is `hf_repo`+`tag` instead of
   * `model_id` in the request, and the resulting pull-state key is the real
   * `<normalized-repo>:<tag>` shape `resolve_hf_pull_target` (Rust) builds.
   */
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
    client.call("hf_pull_model", { hf_repo: repo, tag }).catch((e: Error) => {
      setPullStates((prev) => ({ ...prev, [key]: { phase: "failed", error: e.message } }));
    });
  }, [client, customRepo, customTag]);

  /**
   * LM Studio's own curated-pull path -- the direct sibling of `pullModel`,
   * calling `lmstudio_pull_model` (driving `lms get`) instead of
   * `hf_pull_model` (driving `ollama pull`), writing into the separate
   * `lmPullStates` map so it can never collide with an Ollama pull of the
   * same curated model.
   */
  const pullLmModel = useCallback(
    (modelId: string) => {
      setLmPullStates((prev) => ({ ...prev, [modelId]: { phase: "pulling", lines: [] } }));
      client.call("lmstudio_pull_model", { model_id: modelId }).catch((e: Error) => {
        setLmPullStates((prev) => ({ ...prev, [modelId]: { phase: "failed", error: e.message } }));
      });
    },
    [client]
  );

  /** The LM Studio sibling of `pullCustomModel` -- same real "user defined
   * model download links" mechanism, driving `lmstudio_pull_model` instead
   * of `hf_pull_model`. */
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
    client.call("lmstudio_pull_model", { hf_repo: repo, tag }).catch((e: Error) => {
      setLmPullStates((prev) => ({ ...prev, [key]: { phase: "failed", error: e.message } }));
    });
  }, [client, customLmRepo, customLmTag]);

  /** Real, direct HTTP download for a curated llama.cpp model -- driving
   * `llamacpp_download_model` instead of a subprocess pull. */
  const downloadLlamacppModel = useCallback(
    (modelId: string) => {
      setLlamacppPullStates((prev) => ({ ...prev, [modelId]: { phase: "downloading", lines: [] } }));
      client.call("llamacpp_download_model", { model_id: modelId }).catch((e: Error) => {
        setLlamacppPullStates((prev) => ({
          ...prev,
          [modelId]: { phase: "failed", error: e.message },
        }));
      });
    },
    [client]
  );

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
    client.call("llamacpp_download_model", { hf_repo: repo, tag }).catch((e: Error) => {
      setLlamacppPullStates((prev) => ({ ...prev, [key]: { phase: "failed", error: e.message } }));
    });
  }, [client, customLlamacppRepo, customLlamacppTag]);

  /**
   * Sets a real, already-downloaded `.gguf` file as Leo's active local
   * provider. Fetches the real current settings first -- `settings_set`'s
   * `gpu_enabled` param is mandatory (no fallback), so this can't just send
   * `leo_provider` alone without first reading what's already saved for
   * GPU offload, or it would risk a confusing required-param error / a
   * real accidental reset if a caller ever loosened that requirement.
   */
  const useAsLlamaCppProvider = useCallback(
    (path: string) => {
      setUseModelStatus(`Setting ${path} as the active model…`);
      client
        .call("settings_get")
        .then((current) => {
          const s = current as { gpu_offload?: { enabled: boolean; layers?: number } };
          return client.call("settings_set", {
            gpu_enabled: s.gpu_offload?.enabled ?? false,
            gpu_layers: s.gpu_offload?.layers,
            leo_provider: { kind: "LlamaCpp", model: path },
          });
        })
        .then(() => setUseModelStatus(`✓ ${path} set as Leo's active local model.`))
        .catch((e: Error) => setUseModelStatus(`Failed: ${e.message}`));
    },
    [client]
  );

  /**
   * Real cancel/stop for an in-flight download (task #268) -- `source` is
   * one of the three real registry namespaces `spartan-backend` uses
   * (`"hf"`/`"lmstudio"`/`"llamacpp"`), `eventId` the exact same id already
   * used to key `pullStates`/`lmPullStates`/`llamacppPullStates`. Only sets
   * the real backend flag here; the resulting `..._failed` event (now
   * carrying `cancelled: true`) is what actually flips the UI state, the
   * same way a genuine network failure already does.
   */
  const cancelDownload = useCallback(
    (source: "hf" | "lmstudio" | "llamacpp", eventId: string) => {
      client.call("model_download_cancel", { source, event_id: eventId }).catch(() => {
        // A real failed cancel request (e.g. the download already finished
        // a moment earlier) is a harmless race, not worth surfacing.
      });
    },
    [client]
  );

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
                {state.phase === "pulling" && (
                  <button className="git-commit-button" onClick={() => cancelDownload("hf", model.id)}>
                    Cancel
                  </button>
                )}
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: state.cancelled ? "var(--text-dim)" : "#e05a4a" }}>
                    {state.cancelled ? "Cancelled" : state.error}
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
              <div style={{ marginTop: 4, display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                {state.phase === "pulling" && (
                  <button className="git-commit-button" onClick={() => cancelDownload("hf", key)}>
                    Cancel
                  </button>
                )}
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: state.cancelled ? "var(--text-dim)" : "#e05a4a" }}>
                    {state.cancelled ? "Cancelled" : state.error}
                  </span>
                )}
                {state.phase === "pulling" && state.lines.length > 0 && (
                  <pre
                    className="mono"
                    style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0", width: "100%" }}
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
                {state.phase === "pulling" && (
                  <button className="git-commit-button" onClick={() => cancelDownload("lmstudio", model.id)}>
                    Cancel
                  </button>
                )}
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: state.cancelled ? "var(--text-dim)" : "#e05a4a" }}>
                    {state.cancelled ? "Cancelled" : state.error}
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
              <div style={{ marginTop: 4, display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                {state.phase === "pulling" && (
                  <button className="git-commit-button" onClick={() => cancelDownload("lmstudio", key)}>
                    Cancel
                  </button>
                )}
                {state.phase === "ready" && (
                  <span className="mono" style={{ fontSize: 10.5, color: "var(--accent)" }}>
                    ✓ ready
                  </span>
                )}
                {state.phase === "failed" && (
                  <span className="mono" style={{ fontSize: 10.5, color: state.cancelled ? "var(--text-dim)" : "#e05a4a" }}>
                    {state.cancelled ? "Cancelled" : state.error}
                  </span>
                )}
                {state.phase === "pulling" && state.lines.length > 0 && (
                  <pre
                    className="mono"
                    style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0", width: "100%" }}
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
                {state.phase === "downloading" && (
                  <button className="git-commit-button" onClick={() => cancelDownload("llamacpp", model.id)}>
                    Cancel
                  </button>
                )}
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
                  <span className="mono" style={{ fontSize: 10.5, color: state.cancelled ? "var(--text-dim)" : "#e05a4a" }}>
                    {state.cancelled ? "Cancelled" : state.error}
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
                {state.phase === "downloading" && (
                  <button className="git-commit-button" onClick={() => cancelDownload("llamacpp", key)}>
                    Cancel
                  </button>
                )}
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
                  <span className="mono" style={{ fontSize: 10.5, color: state.cancelled ? "var(--text-dim)" : "#e05a4a" }}>
                    {state.cancelled ? "Cancelled" : state.error}
                  </span>
                )}
                {state.phase === "downloading" && state.lines.length > 0 && (
                  <pre className="mono" style={{ fontSize: 10, maxHeight: 80, overflowY: "auto", margin: "3px 0 0", width: "100%" }}>
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
