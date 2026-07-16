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
  | { phase: "failed"; error: string };

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
 * `_status`, `hf_list_models`/`hf_pull_model`) that have had zero callers
 * anywhere in either shell since they landed (tasks #128, #138, #139). A
 * direct sibling of `GitPanel.tsx`'s own shape -- one `BackendClient`,
 * `.call()`ed directly, `.onEvent()` subscribed for the async litellm/HF
 * progress events, no new protocol needed since `spartan-devserver`'s
 * dispatcher already answers every one of these methods for real.
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

  useEffect(() => {
    refreshModelStatus();
    refreshLitellmStatus();
    refreshHfModels();
  }, [refreshModelStatus, refreshLitellmStatus, refreshHfModels]);

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
        const { model_id, error } = e.data as { model_id: string; error: string };
        setPullStates((prev) => ({ ...prev, [model_id]: { phase: "failed", error } }));
      }
    });
  }, [client, refreshLitellmStatus]);

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
    </div>
  );
}
