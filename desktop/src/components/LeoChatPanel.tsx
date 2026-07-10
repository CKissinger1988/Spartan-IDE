import React, { useCallback, useEffect, useState } from "react";

interface LeoPlan {
  goal: string;
  approach: string;
  files: string[];
  risk_notes: string;
}

interface PendingCall {
  call_id: string;
  tool: string;
  args: Record<string, unknown>;
}

interface LogEntry {
  kind: "call" | "result" | "rejected" | "done" | "failed";
  text: string;
}

type LeoState =
  | "Idle"
  | "Planning"
  | "AwaitingApproval"
  | "Executing"
  | "Verifying"
  | "Done"
  | "Failed"
  | "Recovering"
  | string;

interface LeoChatPanelProps {
  projectRoot: string;
}

function describeCall(call: PendingCall): string {
  switch (call.tool) {
    case "read_file":
      return `Read file: ${call.args.path}`;
    case "edit_file":
      return `Edit file: ${call.args.path}`;
    case "run_terminal":
      return `Run command: ${call.args.command}`;
    default:
      return `${call.tool}(${JSON.stringify(call.args)})`;
  }
}

/**
 * Real, persistent Leo chat panel -- docked, always visible regardless
 * of which nav screen (`ScreenId`) is active, closing a direct user
 * objection ("Where is my Leo chat panel? Leo still runs the show.")
 * to Leo being completely absent from the new Electron shell after the
 * nav restructuring in §75.60. Unlike the original wgpu shell's own
 * Agent mode (a full-screen view you navigate into and away from, §75.47),
 * this panel is a fixed-width column alongside every screen, matching
 * this project's own already-named "docked, not full-screen" future
 * improvement.
 *
 * Talks to the real `spartan-leo::Agent` state machine via
 * `spartan-backend`'s real `leo_*` IPC methods (§75.61) -- `leo_start_task`
 * returns a fast synchronous ack; the real plan (or a real failure) is
 * a real, unprompted `spartan:event` this panel subscribes to via
 * `window.spartan.onEvent`, since a real local-model plan call can take
 * 20-45s+ and must never block the IPC channel.
 *
 * Since §75.66, once a plan is approved this panel drives the real
 * execute loop too: `requestNextStep` asks the model for the next real
 * tool call (or `task_complete`) over the same async `Event` pattern;
 * every real call -- `read_file`/`edit_file`/`run_terminal` -- is shown
 * to the human and requires an explicit Approve/Reject before it
 * actually runs (`leo_start_task` always constructs its `Agent` with
 * `ApprovalMode::ManualEveryStep`, §9's own non-negotiable default, so
 * there is no auto-run path to skip here).
 */
export default function LeoChatPanel({ projectRoot }: LeoChatPanelProps): React.ReactElement {
  const [agentState, setAgentState] = useState<LeoState>("Idle");
  const [plan, setPlan] = useState<LeoPlan | null>(null);
  const [task, setTask] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pendingCall, setPendingCall] = useState<PendingCall | null>(null);
  const [thinking, setThinking] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [summary, setSummary] = useState<string | null>(null);

  const requestNextStep = useCallback(async () => {
    setThinking(true);
    try {
      await window.spartan.call("leo_next_step");
    } catch (e) {
      setThinking(false);
      setError((e as Error).message);
    }
  }, []);

  useEffect(() => {
    window.spartan
      .call("leo_status")
      .then((result) => {
        // Defensive: a malformed/unexpected response (or a backend that
        // doesn't implement this method at all, e.g. a future headless
        // test harness) must never crash this panel -- found live via a
        // Playwright mock that didn't implement `leo_status`, exposing
        // that an undefined `state` reached `.toLowerCase()` below.
        const r = result as
          | { state?: LeoState; plan?: LeoPlan | null; pending_call?: PendingCall | null }
          | undefined;
        setAgentState(r?.state ?? "Idle");
        setPlan(r?.plan ?? null);
        setPendingCall(r?.pending_call ?? null);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "leo_plan_ready") {
        setPlan(data as LeoPlan);
        setAgentState("AwaitingApproval");
        setError(null);
      } else if (event === "leo_plan_failed") {
        setError((data as { error: string }).error);
        setAgentState("Failed");
      } else if (event === "leo_action_proposed") {
        const call = data as PendingCall;
        setThinking(false);
        setPendingCall(call);
        setLog((prev) => [...prev, { kind: "call", text: describeCall(call) }]);
      } else if (event === "leo_execute_done") {
        setThinking(false);
        setPendingCall(null);
        setAgentState("Done");
        const s = (data as { summary: string }).summary;
        setSummary(s);
        setLog((prev) => [...prev, { kind: "done", text: s }]);
      } else if (event === "leo_execute_failed") {
        setThinking(false);
        setPendingCall(null);
        setAgentState("Failed");
        const e = (data as { error: string }).error;
        setError(e);
        setLog((prev) => [...prev, { kind: "failed", text: e }]);
      }
    });
    return unsubscribe;
  }, []);

  const submitTask = useCallback(async () => {
    if (!task.trim()) return;
    setError(null);
    setPlan(null);
    setPendingCall(null);
    setLog([]);
    setSummary(null);
    setAgentState("Planning");
    try {
      await window.spartan.call("leo_start_task", { task, project_root: projectRoot });
    } catch (e) {
      setError((e as Error).message);
      setAgentState("Failed");
    }
  }, [task, projectRoot]);

  const approve = useCallback(async () => {
    try {
      const result = (await window.spartan.call("leo_approve_plan")) as { state: LeoState };
      setAgentState(result.state);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [requestNextStep]);

  const reject = useCallback(async () => {
    try {
      const result = (await window.spartan.call("leo_reject_plan")) as { state: LeoState };
      setAgentState(result.state);
      setPlan(null);
      setTask("");
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

  const approveCall = useCallback(async () => {
    if (!pendingCall) return;
    try {
      const result = (await window.spartan.call("leo_approve_call")) as {
        ok: boolean;
        result?: { kind: string; content?: string; path?: string; bytes?: number; stdout?: string };
        error?: string;
      };
      setPendingCall(null);
      const text = result.ok
        ? result.result?.kind === "file_content"
          ? `Read ${(result.result.content ?? "").length} chars`
          : result.result?.kind === "file_written"
            ? `Wrote ${result.result.bytes} bytes to ${result.result.path}`
            : `Ran command (exit shown in log)`
        : `Failed: ${result.error}`;
      setLog((prev) => [...prev, { kind: "result", text }]);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [pendingCall, requestNextStep]);

  const rejectCall = useCallback(async () => {
    try {
      await window.spartan.call("leo_reject_call");
      setPendingCall(null);
      setLog((prev) => [...prev, { kind: "rejected", text: "Rejected -- asking Leo to reconsider" }]);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [requestNextStep]);

  return (
    <div className="leo-panel">
      <div className="leo-header mono">
        <span className="leo-title">LEO</span>
        <span className={`leo-state leo-state-${agentState.toLowerCase()}`}>{agentState}</span>
      </div>

      <div className="leo-body">
        {plan && agentState === "AwaitingApproval" && (
          <div className="leo-plan">
            <div className="leo-plan-field">
              <span className="leo-plan-label">Goal</span>
              <p>{plan.goal}</p>
            </div>
            <div className="leo-plan-field">
              <span className="leo-plan-label">Approach</span>
              <p>{plan.approach}</p>
            </div>
            <div className="leo-plan-field">
              <span className="leo-plan-label">Files</span>
              <ul className="mono">
                {plan.files.map((f) => (
                  <li key={f}>{f}</li>
                ))}
              </ul>
            </div>
            <div className="leo-plan-field">
              <span className="leo-plan-label">Risk notes</span>
              <p>{plan.risk_notes}</p>
            </div>
            <div className="leo-plan-actions">
              <button className="leo-btn leo-btn-approve" onClick={approve}>
                Approve
              </button>
              <button className="leo-btn leo-btn-reject" onClick={reject}>
                Reject
              </button>
            </div>
          </div>
        )}

        {agentState === "Planning" && (
          <div className="leo-status-message mono">Leo is planning...</div>
        )}

        {error && <div className="leo-error mono">{error}</div>}

        {(agentState === "Executing" || agentState === "Verifying") && (
          <div className="leo-execute">
            {log.length > 0 && (
              <div className="leo-log">
                {log.map((entry, i) => (
                  <div key={i} className={`leo-log-entry leo-log-${entry.kind} mono`}>
                    {entry.text}
                  </div>
                ))}
              </div>
            )}

            {pendingCall && (
              <div className="leo-pending-call">
                <div className="leo-pending-call-desc mono">{describeCall(pendingCall)}</div>
                {pendingCall.tool === "edit_file" && (
                  <pre className="leo-pending-call-content mono">
                    {String(pendingCall.args.content ?? "")}
                  </pre>
                )}
                <div className="leo-plan-actions">
                  <button className="leo-btn leo-btn-approve" onClick={approveCall}>
                    Approve
                  </button>
                  <button className="leo-btn leo-btn-reject" onClick={rejectCall}>
                    Reject
                  </button>
                </div>
              </div>
            )}

            {thinking && !pendingCall && (
              <div className="leo-status-message mono">Leo is thinking about the next step...</div>
            )}
          </div>
        )}

        {agentState === "Done" && summary && (
          <div className="leo-summary mono">
            <span className="leo-plan-label">Done</span>
            <p>{summary}</p>
          </div>
        )}
      </div>

      <div className="leo-input-row">
        <textarea
          className="leo-input mono"
          placeholder="Ask Leo to do something..."
          value={task}
          onChange={(e) => setTask(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              submitTask();
            }
          }}
          disabled={agentState === "Planning" || agentState === "Executing" || agentState === "Verifying"}
        />
        <button
          className="leo-btn leo-btn-send"
          onClick={submitTask}
          disabled={
            agentState === "Planning" ||
            agentState === "Executing" ||
            agentState === "Verifying" ||
            !task.trim()
          }
        >
          Send
        </button>
      </div>
    </div>
  );
}
