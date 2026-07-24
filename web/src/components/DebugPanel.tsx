import React, { useState } from "react";

/** One watch/REPL expression plus its most recent evaluation result -- a
 * direct port of `desktop/src/components/DebugPanel.tsx`'s own `WatchEntry`
 * (duplicated, not shared, since these are two separate npm projects).
 * `value`/`error` are mutually exclusive; `pending` means in flight; all
 * absent means "not evaluated yet" (the session isn't stopped). */
export interface WatchEntry {
  expression: string;
  value?: string;
  error?: string;
  pending?: boolean;
}

/** Mirrors `spartan_dap::{DapStopped, DapFrame, DapVariable}`'s real,
 * unmodified serde field names (no `rename_all` on the Rust side) --
 * exactly what arrives inside a real `dap_stopped` event's `data.stopped`
 * field. The same type `desktop/src/components/DebugPanel.tsx` already
 * defines, duplicated here rather than imported since these are two
 * separate npm projects with no shared package between them. */
export interface DapStoppedInfo {
  thread_id: number;
  reason: string;
  frame: { name: string; line: number } | null;
  variables: { name: string; value: string }[];
}

/** One real DAP `output` event (task #275) -- a logpoint firing, or the
 * debuggee's own real stdout/stderr, both relayed through the identical
 * `dap_output` backend event with no distinguishing marker between them
 * (a real, live-confirmed `debugpy`/`lldb-dap` finding -- see
 * `spartan_dap::DapUpdate::Output`'s own doc comment). The same type
 * `desktop/src/components/DebugPanel.tsx` already defines, duplicated
 * here rather than imported since these are two separate npm projects. */
export interface OutputEntry {
  category: string;
  text: string;
}

export type DapSessionStatus = "launching" | "stopped" | "exited" | "error" | "build_failed";

export interface DapSessionState {
  sessionId: number;
  status: DapSessionStatus;
  stopped?: DapStoppedInfo;
  message?: string;
}

interface DebugPanelProps {
  hasFile: boolean;
  session: DapSessionState | null;
  onLaunch: () => void;
  onContinue: () => void;
  onStepOver: () => void;
  onStepInto: () => void;
  onStop: () => void;
  /** Real watch/REPL expressions + latest results (§250); the App owns them
   * and re-evaluates against the real session on every stop. */
  watches?: WatchEntry[];
  onAddWatch?: (expression: string) => void;
  onRemoveWatch?: (expression: string) => void;
  /** Real DAP output (logpoints + the debuggee's own real stdout/stderr,
   * task #275). The App owns the accumulated log per session (reset on
   * every fresh launch); absent means the feature isn't wired for this
   * render. Rendered regardless of `isLive` -- a real, deliberate choice
   * (not the initial one): a user needs to review the debuggee's own
   * final output most right after it exits, not have it disappear the
   * instant the session leaves the live states. */
  outputLog?: OutputEntry[];
}

/**
 * Real, compact debug toolbar + stack/variables display -- a direct port
 * of `desktop/src/components/DebugPanel.tsx` onto `BackendClient.call`
 * (reached over the real WebSocket transport, §75.88, instead of
 * Electron IPC), extending task #132's DAP wiring to this app for the
 * first time. `spartan-devserver` already falls every unrecognized
 * method (including every real `dap_*` one) through to
 * `spartan_backend::handle_request` unchanged, so no backend or protocol
 * change was needed for this -- purely a web/-side UI addition.
 */
export default function DebugPanel({
  hasFile,
  session,
  onLaunch,
  onContinue,
  onStepOver,
  onStepInto,
  onStop,
  watches,
  onAddWatch,
  onRemoveWatch,
  outputLog,
}: DebugPanelProps): React.ReactElement | null {
  const [watchDraft, setWatchDraft] = useState("");
  if (!hasFile) return null;

  const isStopped = session?.status === "stopped";
  const isLive = session?.status === "launching" || session?.status === "stopped";

  const submitWatch = () => {
    const expr = watchDraft.trim();
    if (expr && onAddWatch) {
      onAddWatch(expr);
      setWatchDraft("");
    }
  };

  return (
    <div className="debug-panel mono">
      <div className="debug-toolbar">
        {!isLive ? (
          <button className="debug-btn debug-btn-primary" onClick={onLaunch} title="Start Debugging">
            ▶ Debug
          </button>
        ) : (
          <>
            <button
              className="debug-btn"
              onClick={onContinue}
              disabled={!isStopped}
              title="Continue"
            >
              ⏵ Continue
            </button>
            <button
              className="debug-btn"
              onClick={onStepOver}
              disabled={!isStopped}
              title="Step Over"
            >
              ⤵ Step Over
            </button>
            <button
              className="debug-btn"
              onClick={onStepInto}
              disabled={!isStopped}
              title="Step Into"
            >
              ⤷ Step Into
            </button>
            <button className="debug-btn debug-btn-stop" onClick={onStop} title="Stop">
              ⏹ Stop
            </button>
          </>
        )}
        {session && (
          <span className={`debug-status debug-status-${session.status}`}>
            {session.status === "launching" && "Launching..."}
            {session.status === "stopped" &&
              `Stopped: ${session.stopped?.reason ?? "?"} at line ${session.stopped?.frame?.line ?? "?"}`}
            {session.status === "exited" && "Program exited"}
            {session.status === "error" && `Error: ${session.message}`}
            {session.status === "build_failed" && `Build failed: ${session.message}`}
          </span>
        )}
      </div>
      {isStopped && session?.stopped && session.stopped.variables.length > 0 && (
        <div className="debug-variables">
          {session.stopped.variables.map((v) => (
            <span key={v.name} className="debug-variable">
              <span className="debug-variable-name">{v.name}</span>
              <span className="debug-variable-eq"> = </span>
              <span className="debug-variable-value">{v.value}</span>
            </span>
          ))}
        </div>
      )}
      {isLive && onAddWatch && (
        <div className="debug-watches">
          <div className="debug-watches-header">
            <span className="debug-watches-title">WATCH</span>
            <input
              className="debug-watch-input"
              value={watchDraft}
              placeholder={isStopped ? "expression (e.g. total * 2)" : "add a watch…"}
              onChange={(e) => setWatchDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  submitWatch();
                }
              }}
            />
          </div>
          {watches && watches.length > 0 && (
            <div className="debug-watch-list">
              {watches.map((w) => (
                <div key={w.expression} className="debug-watch-row">
                  <span className="debug-watch-expr">{w.expression}</span>
                  <span className="debug-watch-eq"> = </span>
                  <span className={`debug-watch-result${w.error ? " debug-watch-error" : ""}`}>
                    {w.pending
                      ? "…"
                      : w.error
                        ? w.error
                        : w.value !== undefined
                          ? w.value
                          : "—"}
                  </span>
                  {onRemoveWatch && (
                    <button
                      className="debug-watch-remove"
                      title="Remove watch"
                      onClick={() => onRemoveWatch(w.expression)}
                    >
                      ×
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
      {outputLog && outputLog.length > 0 && (
        <div className="debug-output">
          <div className="debug-output-title">OUTPUT</div>
          <div className="debug-output-log">
            {outputLog.map((entry, i) => (
              <div key={i} className={`debug-output-line debug-output-${entry.category}`}>
                {entry.text}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
