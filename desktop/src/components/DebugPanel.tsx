import React, { useState } from "react";

/** One watch/REPL expression plus its most recent evaluation result. The
 * App owns the list + evaluation (a watch is re-evaluated against the real
 * DAP session on every stop); this component only renders it and reports
 * add/remove. `value`/`error` are mutually exclusive; `pending` means an
 * evaluation is in flight; all three absent means "not evaluated yet"
 * (e.g. the session is running, not stopped). */
export interface WatchEntry {
  expression: string;
  value?: string;
  error?: string;
  pending?: boolean;
}

/** Mirrors `spartan_dap::{DapStopped, DapFrame, DapVariable}`'s real,
 * unmodified serde field names (no `rename_all` on the Rust side) --
 * exactly what arrives inside a real `dap_stopped` event's `data.stopped`
 * field. `frame.line` is a real, 1-indexed DAP-spec line number, matching
 * the editor gutter's own displayed line numbers directly. */
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
 * `spartan_dap::DapUpdate::Output`'s own doc comment). */
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
  /** Real watch/REPL expressions + their latest evaluation results (§250).
   * The App owns them and re-evaluates against the real session on every
   * stop; absent means the watch feature isn't wired for this render. */
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
 * Real, compact debug toolbar + stack/variables display, closing the
 * desktop-UI half of task #132 (DAP wiring) -- the first time real
 * debugging is reachable from either Electron-based shell (it has only
 * ever existed in the reference wgpu shell's F5/F9/F10/F11 keybindings).
 * Deliberately a single inline bar rather than a docked panel, matching
 * this codebase's own established "small, honest first increment" style
 * (`StatusBar.tsx`, `GitPanel.tsx`'s own compact list) rather than a
 * larger docked-panel redesign that isn't this pass's scope.
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
                  <span
                    className={`debug-watch-result${w.error ? " debug-watch-error" : ""}`}
                  >
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
              <div
                key={i}
                className={`debug-output-line debug-output-${entry.category}`}
              >
                {entry.text}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
