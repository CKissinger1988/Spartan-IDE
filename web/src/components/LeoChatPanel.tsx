import React, { useCallback, useEffect, useRef, useState } from "react";
import type { BackendClient } from "../backendClient";

/**
 * Real Leo chat panel for `web/`'s backend-connected mode -- closes the
 * gap this project's own `docs/FUTURE_FEATURES.md` named ("Web app:
 * LSP/DAP/Leo/git in the pure client-side mode... currently these only
 * work in backend-connected mode") for the one piece that was still
 * missing even in *backend-connected* mode: unlike LSP diagnostics/hover/
 * completion/definition/etc. and the Git panel, `web/` never had any Leo
 * UI at all, confirmed by grep before writing this file (the only prior
 * `web/` reference to "Leo" was `LeoProviderKind` in `ModelsPanel.tsx`'s
 * settings dropdown, not an actual chat/task/execute-loop surface).
 *
 * This is a direct, close port of `desktop/src/components/LeoChatPanel.tsx`
 * -- same real `spartan-leo::Agent` state machine, same real `leo_*`
 * dispatch methods (`leo_start_task`/`leo_approve_plan`/`leo_reject_plan`/
 * `leo_next_step`/`leo_approve_call`/`leo_reject_call`/`leo_cancel`/
 * `leo_retry`/`leo_status`/`leo_session_history`), all of which are already
 * real, generic `spartan-backend` methods with zero method allowlist to
 * extend -- `web/`'s own `BackendClient` reaches them exactly the same way
 * it already reaches `git_status`/`lsp_hover`/`dap_launch`. The only real
 * differences from the desktop copy: this component takes the already-
 * connected `BackendClient` instance directly (rather than reading a
 * global `window.spartan`), and `BackendClient.onEvent`'s own real
 * callback shape is a single `{event, data}` object (`client.onEvent((e) =>
 * ...)`), not Electron's two-argument `window.spartan.onEvent(event, data)`
 * -- matching every other `web/` component that already subscribes to
 * backend events (e.g. `App.tsx`'s own DAP/Android handlers).
 */
interface SpeechRecognitionResultLike {
  isFinal: boolean;
  0: { transcript: string };
}
interface SpeechRecognitionEventLike {
  resultIndex: number;
  results: { length: number; [index: number]: SpeechRecognitionResultLike };
}
interface SpeechRecognitionErrorEventLike {
  error: string;
}
interface SpeechRecognitionLike {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  start: () => void;
  stop: () => void;
  onresult: ((event: SpeechRecognitionEventLike) => void) | null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null;
  onend: (() => void) | null;
}
type SpeechRecognitionCtor = new () => SpeechRecognitionLike;

/** Real, honest feature detection -- matches `desktop/`'s own copy exactly.
 * A plain browser tab (this is genuinely what `web/` is) has at least as
 * good a claim to these standard Web APIs as Electron's bundled Chromium
 * does, so this isn't a hypothetical port -- the same real mic/speech
 * synthesis path applies unchanged. */
function getSpeechRecognitionCtor(): SpeechRecognitionCtor | null {
  const w = window as unknown as {
    SpeechRecognition?: SpeechRecognitionCtor;
    webkitSpeechRecognition?: SpeechRecognitionCtor;
  };
  return w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null;
}

const VOICE_OUTPUT_STORAGE_KEY = "spartan.leo.voiceOutputEnabled";

/** Same curated, hand-written array as `desktop/`'s own copy (no Gemini
 * CLI code read or copied -- see that file's own doc comment for the
 * full account of what this behavior is and isn't based on). */
const LEO_THOUGHTS: readonly string[] = [
  "Reticulating splines, mostly out of spite...",
  "Silently judging your variable names...",
  "Definitely not just guessing here...",
  "Pretending this is harder than it is, for effect...",
  "Consulting the ghosts of stack traces past...",
  "Counting the semicolons you forgot...",
  "Summoning my inner grumpy senior engineer...",
  "Weighing the pros and cons of just winging it...",
  "Double-checking so I don't look dumb later...",
  "Muttering about tech debt under my breath...",
  "Politely ignoring that TODO comment I just saw...",
  "Crunching numbers, mostly for dramatic effect...",
  "Deciding whether this deserves a witty remark...",
  "Recalling every bug I've ever fixed, for inspiration...",
  "Resisting the urge to rewrite everything...",
  "Pretending I've never seen a bug this weird before...",
  "Buying time so this looks like real effort...",
  "Rolling my eyes at this codebase, lovingly...",
];

function useRandomThought(active: boolean): string | null {
  const [index, setIndex] = useState(0);

  useEffect(() => {
    if (!active) return;
    setIndex(Math.floor(Math.random() * LEO_THOUGHTS.length));
    const interval = setInterval(() => {
      setIndex((prev) => {
        if (LEO_THOUGHTS.length <= 1) return prev;
        let next = Math.floor(Math.random() * LEO_THOUGHTS.length);
        if (next === prev) next = (next + 1) % LEO_THOUGHTS.length;
        return next;
      });
    }, 2500);
    return () => clearInterval(interval);
  }, [active]);

  return active ? LEO_THOUGHTS[index] : null;
}

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
  diff?: string;
}

interface LogEntry {
  kind: "call" | "result" | "rejected" | "done" | "failed" | "auto";
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
  client: BackendClient;
}

interface LeoHistoryEntry {
  task: string;
  outcome: string;
  summary: string | null;
  error: string | null;
  unix_timestamp: number;
}

/** Matches `GitPanel.tsx`'s own `formatAge` verbatim -- this project's
 * established per-component-copy discipline, not a shared package. */
function formatAge(unixSeconds: number): string {
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

function describeCall(call: PendingCall): string {
  switch (call.tool) {
    case "read_file":
      return `Read file: ${call.args.path}`;
    case "edit_file":
      return `Edit file: ${call.args.path}`;
    case "run_terminal":
      return `Run command: ${call.args.command}`;
    case "search_files":
      return call.args.path
        ? `Search for "${call.args.pattern}" in ${call.args.path}`
        : `Search project for "${call.args.pattern}"`;
    case "list_directory":
      return call.args.path ? `List directory: ${call.args.path}` : "List project root";
    default:
      return `${call.tool}(${JSON.stringify(call.args)})`;
  }
}

function DiffView({ diff }: { diff: string }): React.ReactElement {
  const lines = diff.split("\n").filter((_, i, arr) => !(i === arr.length - 1 && arr[i] === ""));
  return (
    <pre className="leo-diff mono">
      {lines.map((line, i) => {
        const kind = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
        return (
          <div key={i} className={`leo-diff-line leo-diff-${kind}`}>
            {line || " "}
          </div>
        );
      })}
    </pre>
  );
}

export default function LeoChatPanel({ client }: LeoChatPanelProps): React.ReactElement {
  const [agentState, setAgentState] = useState<LeoState>("Idle");
  const [plan, setPlan] = useState<LeoPlan | null>(null);
  const [task, setTask] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pendingCall, setPendingCall] = useState<PendingCall | null>(null);
  const [thinking, setThinking] = useState(false);
  const [log, setLog] = useState<LogEntry[]>([]);
  const [summary, setSummary] = useState<string | null>(null);
  const [memorySaved, setMemorySaved] = useState<boolean | null>(null);

  const [historyOpen, setHistoryOpen] = useState(false);
  const [history, setHistory] = useState<LeoHistoryEntry[] | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);

  const showingRandomThought = agentState === "Planning" || (thinking && !pendingCall);
  const randomThought = useRandomThought(showingRandomThought);

  const [voiceInputSupported] = useState(() => getSpeechRecognitionCtor() !== null);
  const [voiceOutputSupported] = useState(
    () => typeof window !== "undefined" && "speechSynthesis" in window
  );
  const [listening, setListening] = useState(false);
  const [voiceOutputEnabled, setVoiceOutputEnabled] = useState(
    () => typeof window !== "undefined" && window.localStorage.getItem(VOICE_OUTPUT_STORAGE_KEY) === "1"
  );
  const recognitionRef = useRef<SpeechRecognitionLike | null>(null);

  const speak = useCallback(
    (text: string) => {
      if (!voiceOutputEnabled || !voiceOutputSupported || !text) return;
      const synth = window.speechSynthesis;
      synth.cancel();
      synth.speak(new SpeechSynthesisUtterance(text));
    },
    [voiceOutputEnabled, voiceOutputSupported]
  );

  const toggleVoiceOutput = useCallback(() => {
    setVoiceOutputEnabled((prev) => {
      const next = !prev;
      window.localStorage.setItem(VOICE_OUTPUT_STORAGE_KEY, next ? "1" : "0");
      if (!next) window.speechSynthesis?.cancel();
      return next;
    });
  }, []);

  const toggleListening = useCallback(() => {
    if (listening) {
      recognitionRef.current?.stop();
      return;
    }
    const Ctor = getSpeechRecognitionCtor();
    if (!Ctor) return;
    const recognition = new Ctor();
    recognition.lang = "en-US";
    recognition.continuous = true;
    recognition.interimResults = false;
    recognition.onresult = (event) => {
      let transcript = "";
      for (let i = event.resultIndex; i < event.results.length; i++) {
        const result = event.results[i];
        if (result.isFinal) transcript += result[0].transcript;
      }
      if (transcript.trim()) {
        setTask((prev) => (prev ? `${prev} ${transcript}`.trim() : transcript.trim()));
      }
    };
    recognition.onerror = (event) => {
      setError(`Voice input error: ${event.error}`);
      setListening(false);
    };
    recognition.onend = () => setListening(false);
    recognitionRef.current = recognition;
    recognition.start();
    setListening(true);
  }, [listening]);

  useEffect(() => {
    return () => {
      recognitionRef.current?.stop();
      window.speechSynthesis?.cancel();
    };
  }, []);

  const requestNextStep = useCallback(async () => {
    setThinking(true);
    try {
      await client.call("leo_next_step");
    } catch (e) {
      setThinking(false);
      setError((e as Error).message);
    }
  }, [client]);

  useEffect(() => {
    client
      .call("leo_status")
      .then((result) => {
        // Same defensive shape-check as `desktop/`'s own copy -- a
        // malformed/unexpected response must never crash this panel.
        const r = result as
          | { state?: LeoState; plan?: LeoPlan | null; pending_call?: PendingCall | null }
          | undefined;
        setAgentState(r?.state ?? "Idle");
        setPlan(r?.plan ?? null);
        setPendingCall(r?.pending_call ?? null);
      })
      .catch(() => {});
  }, [client]);

  useEffect(() => {
    const unsubscribe = client.onEvent(({ event, data }) => {
      if (event === "leo_plan_ready") {
        const readyPlan = data as LeoPlan;
        setPlan(readyPlan);
        setAgentState("AwaitingApproval");
        setError(null);
        speak(`Leo has a plan: ${readyPlan.goal}`);
      } else if (event === "leo_plan_failed") {
        const failMessage = (data as { error: string }).error;
        setError(failMessage);
        setAgentState("Failed");
        speak(`Leo ran into an error: ${failMessage}`);
      } else if (event === "leo_action_proposed") {
        const call = data as PendingCall;
        setThinking(false);
        setPendingCall(call);
        setLog((prev) => [...prev, { kind: "call", text: describeCall(call) }]);
      } else if (event === "leo_auto_step") {
        const step = data as PendingCall;
        setLog((prev) => [
          ...prev,
          { kind: "auto", text: `Auto-approved: ${describeCall(step)}` },
        ]);
      } else if (event === "leo_execute_done") {
        setThinking(false);
        setPendingCall(null);
        setAgentState("Done");
        const { summary: s, memory_saved } = data as { summary: string; memory_saved: boolean };
        setSummary(s);
        setMemorySaved(memory_saved);
        setLog((prev) => [...prev, { kind: "done", text: s }]);
        speak(s);
      } else if (event === "leo_execute_failed") {
        setThinking(false);
        setPendingCall(null);
        setAgentState("Failed");
        const e = (data as { error: string }).error;
        setError(e);
        setLog((prev) => [...prev, { kind: "failed", text: e }]);
        speak(`Leo ran into an error: ${e}`);
      }
    });
    return unsubscribe;
  }, [client, speak]);

  const submitTask = useCallback(async () => {
    if (!task.trim()) return;
    setError(null);
    setPlan(null);
    setPendingCall(null);
    setLog([]);
    setSummary(null);
    setMemorySaved(null);
    setAgentState("Planning");
    try {
      await client.call("leo_start_task", { task, project_root: client.projectRoot });
    } catch (e) {
      setError((e as Error).message);
      setAgentState("Failed");
    }
  }, [task, client]);

  const approve = useCallback(async () => {
    try {
      const result = (await client.call("leo_approve_plan")) as { state: LeoState };
      setAgentState(result.state);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [client, requestNextStep]);

  const reject = useCallback(async () => {
    try {
      const result = (await client.call("leo_reject_plan")) as { state: LeoState };
      setAgentState(result.state);
      setPlan(null);
      setTask("");
    } catch (e) {
      setError((e as Error).message);
    }
  }, [client]);

  const cancelTask = useCallback(async () => {
    try {
      const result = (await client.call("leo_cancel")) as { state: LeoState };
      setAgentState(result.state);
      setPlan(null);
      setPendingCall(null);
      setThinking(false);
      setLog([]);
      setSummary(null);
      setMemorySaved(null);
      setError(null);
      setTask("");
    } catch (e) {
      setError((e as Error).message);
    }
  }, [client]);

  const retryTask = useCallback(async () => {
    setError(null);
    try {
      const result = (await client.call("leo_retry")) as { state: LeoState };
      setAgentState(result.state);
      setLog((prev) => [...prev, { kind: "auto", text: "Retrying failed task..." }]);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [client, requestNextStep]);

  const approveCall = useCallback(async () => {
    if (!pendingCall) return;
    try {
      const result = (await client.call("leo_approve_call")) as {
        ok: boolean;
        result?: {
          kind: string;
          content?: string;
          path?: string;
          bytes?: number;
          matches?: unknown[];
          entries?: unknown[];
        };
        error?: string;
      };
      setPendingCall(null);
      let text: string;
      if (!result.ok) {
        text = `Failed: ${result.error}`;
      } else {
        switch (result.result?.kind) {
          case "file_content":
            text = `Read ${(result.result.content ?? "").length} chars`;
            break;
          case "file_written":
            text = `Wrote ${result.result.bytes} bytes to ${result.result.path}`;
            break;
          case "search_matches":
            text = `Found ${result.result.matches?.length ?? 0} match(es)`;
            break;
          case "directory_listing":
            text = `Listed ${result.result.entries?.length ?? 0} entries`;
            break;
          default:
            text = "Ran command (exit shown in log)";
        }
      }
      setLog((prev) => [...prev, { kind: "result", text }]);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [client, pendingCall, requestNextStep]);

  const rejectCall = useCallback(async () => {
    try {
      await client.call("leo_reject_call");
      setPendingCall(null);
      setLog((prev) => [...prev, { kind: "rejected", text: "Rejected -- asking Leo to reconsider" }]);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [client, requestNextStep]);

  const toggleHistory = useCallback(() => {
    if (historyOpen) {
      setHistoryOpen(false);
      setHistoryError(null);
      return;
    }
    setHistoryOpen(true);
    setHistoryError(null);
    client
      .call("leo_session_history")
      .then((result) => {
        const r = result as { entries?: LeoHistoryEntry[] } | undefined;
        setHistory(r?.entries ?? []);
      })
      .catch((e: Error) => setHistoryError(e.message));
  }, [client, historyOpen]);

  return (
    <div className="leo-panel">
      <div className="leo-header mono">
        <span className="leo-title">LEO</span>
        {voiceOutputSupported && (
          <button
            className={`leo-btn leo-btn-voice-toggle${voiceOutputEnabled ? " leo-btn-voice-on" : ""}`}
            onClick={toggleVoiceOutput}
            title={voiceOutputEnabled ? "Voice responses on" : "Voice responses off"}
          >
            {voiceOutputEnabled ? "\u{1F50A}" : "\u{1F507}"}
          </button>
        )}
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
              <button className="leo-btn leo-btn-approve sf-chamfer-sm" onClick={approve}>
                Approve
              </button>
              <button className="leo-btn leo-btn-reject" onClick={reject}>
                Reject
              </button>
            </div>
          </div>
        )}

        {agentState === "Planning" && (
          <div className="leo-status-message mono">
            Leo is planning...
            {randomThought && <div className="leo-random-thought mono">{randomThought}</div>}
            <button className="leo-btn leo-btn-cancel" onClick={cancelTask}>
              Cancel
            </button>
          </div>
        )}

        {error && <div className="leo-error mono">{error}</div>}

        {agentState === "Failed" && (
          <div className="leo-status-message mono">
            <button className="leo-btn leo-btn-approve sf-chamfer-sm" onClick={retryTask}>
              Retry
            </button>
          </div>
        )}

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
                {pendingCall.tool === "edit_file" &&
                  (pendingCall.diff ? (
                    <DiffView diff={pendingCall.diff} />
                  ) : (
                    <pre className="leo-pending-call-content mono">
                      {String(pendingCall.args.content ?? "")}
                    </pre>
                  ))}
                <div className="leo-plan-actions">
                  <button className="leo-btn leo-btn-approve sf-chamfer-sm" onClick={approveCall}>
                    Approve
                  </button>
                  <button className="leo-btn leo-btn-reject" onClick={rejectCall}>
                    Reject
                  </button>
                </div>
              </div>
            )}

            {thinking && !pendingCall && (
              <div className="leo-status-message mono">
                Leo is thinking about the next step...
                {randomThought && <div className="leo-random-thought mono">{randomThought}</div>}
              </div>
            )}

            <button className="leo-btn leo-btn-cancel" onClick={cancelTask}>
              Cancel Task
            </button>
          </div>
        )}

        {agentState === "Done" && summary && (
          <div className="leo-summary mono">
            <span className="leo-plan-label">Done</span>
            <p>{summary}</p>
            {memorySaved !== null && (
              <p className="leo-memory-note">
                {memorySaved ? "Saved a note to project memory." : "Could not save to project memory."}
              </p>
            )}
          </div>
        )}

        <div
          className="git-section-label mono"
          onClick={toggleHistory}
          style={{ cursor: "pointer" }}
          title="Past Leo tasks this session"
        >
          History {historyOpen ? "▾" : "▸"}
        </div>
        {historyOpen && (
          <div className="git-section">
            {historyError && <div className="git-panel-empty mono">{historyError}</div>}
            {history === null && !historyError && (
              <div className="git-panel-empty mono">Loading history…</div>
            )}
            {history?.length === 0 && (
              <div className="git-panel-empty mono">No past tasks yet.</div>
            )}
            {history?.map((entry, i) => (
              <React.Fragment key={i}>
                <div
                  className="git-row"
                  title={`${entry.task}\n${new Date(entry.unix_timestamp * 1000).toLocaleString()}`}
                >
                  <span
                    className={`mono leo-history-outcome leo-history-outcome-${entry.outcome.toLowerCase()}`}
                  >
                    {entry.outcome}
                  </span>
                  <span className="mono git-row-path">{entry.task || "(untitled task)"}</span>
                  <span style={{ opacity: 0.6, whiteSpace: "nowrap", fontSize: 11 }} className="mono">
                    {formatAge(entry.unix_timestamp)}
                  </span>
                </div>
                {(entry.summary || entry.error) && (
                  <p className="leo-history-detail mono">{entry.summary ?? entry.error}</p>
                )}
              </React.Fragment>
            ))}
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
        {voiceInputSupported && (
          <button
            className={`leo-btn leo-btn-mic${listening ? " leo-btn-mic-active" : ""}`}
            onClick={toggleListening}
            disabled={agentState === "Planning" || agentState === "Executing" || agentState === "Verifying"}
            title={listening ? "Stop dictating" : "Dictate task"}
          >
            {listening ? "\u{1F534}" : "\u{1F3A4}"}
          </button>
        )}
        <button
          className="leo-btn leo-btn-send sf-chamfer-sm"
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
