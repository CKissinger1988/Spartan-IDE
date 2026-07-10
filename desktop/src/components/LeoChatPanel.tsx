import React, { useCallback, useEffect, useState } from "react";

interface LeoPlan {
  goal: string;
  approach: string;
  files: string[];
  risk_notes: string;
}

type LeoState = "Idle" | "Planning" | "AwaitingApproval" | "Executing" | "Failed" | string;

interface LeoChatPanelProps {
  projectRoot: string;
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
 */
export default function LeoChatPanel({ projectRoot }: LeoChatPanelProps): React.ReactElement {
  const [agentState, setAgentState] = useState<LeoState>("Idle");
  const [plan, setPlan] = useState<LeoPlan | null>(null);
  const [task, setTask] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    window.spartan
      .call("leo_status")
      .then((result) => {
        // Defensive: a malformed/unexpected response (or a backend that
        // doesn't implement this method at all, e.g. a future headless
        // test harness) must never crash this panel -- found live via a
        // Playwright mock that didn't implement `leo_status`, exposing
        // that an undefined `state` reached `.toLowerCase()` below.
        const r = result as { state?: LeoState; plan?: LeoPlan | null } | undefined;
        setAgentState(r?.state ?? "Idle");
        setPlan(r?.plan ?? null);
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
      }
    });
    return unsubscribe;
  }, []);

  const submitTask = useCallback(async () => {
    if (!task.trim()) return;
    setError(null);
    setPlan(null);
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
    } catch (e) {
      setError((e as Error).message);
    }
  }, []);

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

        {agentState === "Executing" && (
          <div className="leo-status-message mono">
            Plan approved -- a real checkpoint was created. No automated execute/verify loop is
            wired yet (real, named gap: spartan-leo's own execute/verify machinery isn't driven
            from this shell).
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
          disabled={agentState === "Planning"}
        />
        <button
          className="leo-btn leo-btn-send"
          onClick={submitTask}
          disabled={agentState === "Planning" || !task.trim()}
        >
          Send
        </button>
      </div>
    </div>
  );
}
