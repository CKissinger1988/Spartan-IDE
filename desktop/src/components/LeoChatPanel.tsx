import React, { useCallback, useEffect, useRef, useState } from "react";

/**
 * Real §75.71 voice I/O -- second and final "concepts only, rebuilt
 * safely" increment adapted from `CKissinger1988/SpartanAI_Assistant`
 * (see §75.70 for the full scoping discussion). That repo's own
 * "Dynamic Personas & Voice" concept runs local `whisper` STT and
 * `edge-tts` TTS as new Python dependencies; this uses Electron's own
 * Chromium-native Web Speech API instead -- zero new dependencies, and
 * no code ported from the source repo (that repo's `voice.py` was never
 * read in detail; only the README's own feature description was).
 * Deliberately narrow, minimal browser-API typings rather than pulling
 * in `@types/dom-speech-recognition` -- this project's own established
 * "declare only what's actually used" precedent (matching `nav.ts`'s own
 * narrow typing style).
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

/** Real, honest feature detection -- Electron's bundled Chromium usually
 * exposes `webkitSpeechRecognition`, but its actual recognition backend
 * still depends on network reachability to a Google speech service in
 * most builds; this returns `null` (not a fake stub) when unavailable,
 * so the UI can degrade honestly instead of showing a mic button that
 * silently does nothing. */
function getSpeechRecognitionCtor(): SpeechRecognitionCtor | null {
  const w = window as unknown as {
    SpeechRecognition?: SpeechRecognitionCtor;
    webkitSpeechRecognition?: SpeechRecognitionCtor;
  };
  return w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null;
}

const VOICE_OUTPUT_STORAGE_KEY = "spartan.leo.voiceOutputEnabled";

/**
 * Real §75.95 "random thoughts," user-requested ("Leo should show random
 * thoughts similar to Gemini Cli"): Gemini CLI shows a rotating line of
 * playful status text while it's actively working, instead of one static
 * "thinking..." message the whole time. This is that same real UX pattern,
 * built fresh for this app -- a curated, hand-written array (no Gemini CLI
 * code read or copied; only the described *behavior* was the reference),
 * flavored to match Leo's own real §75.95 sarcastic persona (`crates/
 * spartan-leo/src/persona.rs`) rather than Gemini's own neutral tone, so
 * the chat panel's voice matches the model's own system-prompt voice
 * instead of contradicting it.
 */
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

/** Cycles to a real, freshly-random `LEO_THOUGHTS` entry every ~2.5s while
 * `active` is true, and stays `null` (rendered as nothing) otherwise --
 * mirrors this panel's own established "no fake stub while inactive"
 * discipline (`getSpeechRecognitionCtor()`'s own honest-`null` pattern
 * above). Deliberately avoids repeating the immediately-previous thought
 * back-to-back so a short-lived step doesn't visibly "not change." */
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
  /** Real §75.68 diff preview -- only present for `edit_file` proposals,
   * a plain `+`/`-`/` `-prefixed line diff computed server-side against
   * the file's real current content. */
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

/** Real diff rendering -- one `<div>` per real line, colored by its real
 * `+`/`-`/` ` prefix, matching a real, minimal, modern diff view rather
 * than a raw text dump. */
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
  const [memorySaved, setMemorySaved] = useState<boolean | null>(null);

  // Real §75.95 random-thoughts status text -- active exactly while Leo is
  // doing real, unattended work with nothing more specific to show yet:
  // the initial plan-generation call, or between execute-loop steps once
  // a step has been requested but no real proposed call has arrived (or
  // been auto-run) to describe instead.
  const showingRandomThought = agentState === "Planning" || (thinking && !pendingCall);
  const randomThought = useRandomThought(showingRandomThought);

  // Real §75.71 voice I/O state. `voiceOutputEnabled` is a pure renderer
  // preference (not routed through `spartan_settings` -- it has no
  // backend/Leo-behavior effect the way GPU offload or the provider
  // choice do), persisted to `localStorage` since that's the honest,
  // simplest real mechanism for a browser-native UI toggle.
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
        // Real §75.69 auto-approved Safe call -- Leo already ran this
        // itself (AutoApproveSafe mode) with no UI round trip; still
        // logged for real visibility into what it actually did.
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
  }, [speak]);

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

  /** Real §75.73 cancel -- task #58's own named remaining item, "a UI
   * control to interrupt an in-progress planning or execute loop." Real,
   * deliberate scope limit named in `leo_cancel`'s own real backend doc
   * comment applies here too: this cannot forcibly stop a real
   * in-flight model call already running on its own background thread
   * (no cooperative-cancellation channel exists for that yet) -- what it
   * *does* do, and what makes it real rather than cosmetic, is bump the
   * real backend's generation counter so that call's eventual result is
   * discarded rather than silently resurrecting the cancelled task; this
   * panel resets its own local view to a fresh, empty Idle state
   * immediately rather than waiting to see whether that stale result
   * ever arrives. */
  const cancelTask = useCallback(async () => {
    try {
      const result = (await window.spartan.call("leo_cancel")) as { state: LeoState };
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
  }, []);

  /** Real §75.78 retry -- the "Failed -> Recovering -> Executing" loop's
   * last missing piece. Mirrors `approve`'s exact shape: call the real
   * backend transition, adopt whatever state it reports, then
   * immediately ask for the next step, exactly like approving a plan
   * already does. A real `RecoveryExhausted` error surfaces as a plain
   * error message (via the existing `error` state, already rendered
   * unconditionally above) rather than a special-cased UI -- the honest
   * backend message ("start a new task instead") already says what to
   * do next. */
  const retryTask = useCallback(async () => {
    setError(null);
    try {
      const result = (await window.spartan.call("leo_retry")) as { state: LeoState };
      setAgentState(result.state);
      setLog((prev) => [...prev, { kind: "auto", text: "Retrying failed task..." }]);
      requestNextStep();
    } catch (e) {
      setError((e as Error).message);
    }
  }, [requestNextStep]);

  const approveCall = useCallback(async () => {
    if (!pendingCall) return;
    try {
      const result = (await window.spartan.call("leo_approve_call")) as {
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
