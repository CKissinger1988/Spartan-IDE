import React from "react";

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
}: DebugPanelProps): React.ReactElement | null {
  if (!hasFile) return null;

  const isStopped = session?.status === "stopped";
  const isLive = session?.status === "launching" || session?.status === "stopped";

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
    </div>
  );
}
