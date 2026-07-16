import React, { useCallback, useEffect, useState } from "react";
import Sidebar from "./components/Sidebar";
import FileTree from "./components/FileTree";
import GitPanel from "./components/GitPanel";
import TabBar from "./components/TabBar";
import StatusBar from "./components/StatusBar";
import Editor, {
  type EditorPrefs,
  DEFAULT_EDITOR_PREFS,
  type OpenFile,
  type LspDiagnostic,
} from "./components/Editor";
import DebugPanel, { type DapSessionState } from "./components/DebugPanel";
import Placeholder from "./components/Placeholder";
import WorkflowsScreen from "./components/WorkflowsScreen";
import DesignScreen from "./components/DesignScreen";
import ConsoleScreen from "./components/ConsoleScreen";
import SessionsScreen from "./components/SessionsScreen";
import SettingsScreen from "./components/SettingsScreen";
import DevContainersScreen from "./components/DevContainersScreen";
import LeoChatPanel from "./components/LeoChatPanel";
import NewProjectWizard from "./components/NewProjectWizard";
import OnboardingScreen from "./components/OnboardingScreen";
import { applyReduceMotion } from "./reduceMotion";
import { applyTheme, type ThemeName } from "./applyTheme";
import { applyFontFamily } from "./applyFontFamily";
import { NAV, type ScreenId } from "./nav";
import "./app.css";

const ROOT = new URLSearchParams(window.location.search).get("root") ?? "/";

export default function App(): React.ReactElement {
  const [files, setFiles] = useState<OpenFile[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [screen, setScreen] = useState<ScreenId>("editor");
  // Real Ctrl+G sidebar toggle (§75.30's own convention in the original
  // wgpu shell -- one left-rail region, not a second pane, shared between
  // the file tree and the real Source Control panel added in §75.65).
  const [sidebarView, setSidebarView] = useState<"files" | "git">("files");
  const [showNewProjectWizard, setShowNewProjectWizard] = useState(false);
  const [onboardingState, setOnboardingState] = useState<"checking" | "show" | "done">(
    "checking"
  );
  // Real bug fix, found by a code-review pass: `Editor.tsx` used to fetch
  // this exact same settings object independently on its own mount,
  // costing a second, fully redundant IPC round trip to the backend
  // subprocess for data already fetched here a moment earlier. Lifted up
  // and passed down as a prop instead -- `Editor.tsx` still has its own
  // real default if this hasn't resolved yet (e.g. a file opened via a
  // deep link before the very first paint).
  const [editorPrefs, setEditorPrefs] = useState<EditorPrefs>(DEFAULT_EDITOR_PREFS);
  // Real, live LSP diagnostics (§75.6-class backend wiring, closing the
  // desktop/+web/ gap that shell has carried since the Electron pivot),
  // keyed by `doc_id` so switching tabs doesn't lose another open file's
  // diagnostics -- each `lsp_diagnostics` event fully replaces the prior
  // set for that doc_id (the backend always sends the complete current
  // list, never a delta).
  const [diagnosticsByDoc, setDiagnosticsByDoc] = useState<Record<number, LspDiagnostic[]>>({});
  // Real DAP state (§132), both keyed by `doc_id` the same way
  // `diagnosticsByDoc` already is -- breakpoints are 1-indexed line
  // numbers (matching the gutter's own display and the real
  // `dap_launch` `break_lines` param directly, no translation); a
  // session entry exists only while a debug session for that file is
  // live or has just finished (exited/errored), so the toolbar can show
  // its final state before the user dismisses it via Stop or relaunches.
  const [breakpointsByDoc, setBreakpointsByDoc] = useState<Record<number, number[]>>({});
  const [dapSessionByDoc, setDapSessionByDoc] = useState<Record<number, DapSessionState>>({});

  useEffect(() => {
    const unsubscribe = window.spartan.onEvent((event, data) => {
      if (event === "lsp_diagnostics") {
        const { doc_id, diagnostics } = data as { doc_id: number; diagnostics: LspDiagnostic[] };
        setDiagnosticsByDoc((prev) => ({ ...prev, [doc_id]: diagnostics }));
      } else if (event === "lsp_error") {
        // A real, honest server-side condition (handshake never completed,
        // or no diagnostics update arrived in time) -- not a UI-breaking
        // error. Logged for now; a dedicated status surface is real,
        // separate follow-up work.
        console.warn("lsp_error:", data);
      } else if (event === "dap_stopped") {
        const { doc_id, stopped } = data as {
          doc_id: number;
          stopped: DapSessionState["stopped"];
        };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return { ...prev, [doc_id]: { ...existing, status: "stopped", stopped } };
        });
      } else if (event === "dap_exited") {
        const { doc_id } = data as { doc_id: number };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return { ...prev, [doc_id]: { ...existing, status: "exited" } };
        });
      } else if (event === "dap_error") {
        const { doc_id, message } = data as { doc_id: number; message: string };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return { ...prev, [doc_id]: { ...existing, status: "error", message } };
        });
      } else if (event === "dap_build_failed") {
        const { doc_id, diagnostics } = data as { doc_id: number; diagnostics: string[] };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return {
            ...prev,
            [doc_id]: { ...existing, status: "build_failed", message: diagnostics.join("\n") },
          };
        });
      }
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "g") {
        e.preventDefault();
        setSidebarView((v) => (v === "files" ? "git" : "files"));
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // Real §75.76 startup settings read -- applies "reduce motion"
  // immediately (so it's correct even if the user never opens Settings
  // this session; `SettingsScreen.tsx` re-applies it live the moment the
  // toggle changes) and decides whether first-run onboarding should show
  // at all, gated by the real, persisted `onboarding_completed` flag.
  useEffect(() => {
    window.spartan
      .call("settings_get", {})
      .then((result) => {
        const s = result as {
          appearance?: { reduce_motion?: boolean; theme?: ThemeName };
          onboarding_completed?: boolean;
          editor?: {
            font_size?: number;
            tab_size?: number;
            word_wrap?: boolean;
            font_family?: string | null;
          };
        };
        applyReduceMotion(Boolean(s.appearance?.reduce_motion));
        applyTheme(s.appearance?.theme ?? "SpartanDark");
        applyFontFamily(s.editor?.font_family);
        setOnboardingState(s.onboarding_completed ? "done" : "show");
        if (s.editor) {
          setEditorPrefs({
            fontSize: s.editor.font_size ?? DEFAULT_EDITOR_PREFS.fontSize,
            tabSize: s.editor.tab_size ?? DEFAULT_EDITOR_PREFS.tabSize,
            wordWrap: s.editor.word_wrap ?? DEFAULT_EDITOR_PREFS.wordWrap,
          });
        }
      })
      .catch(() => setOnboardingState("done"));
  }, []);

  const openFile = useCallback(
    async (path: string) => {
      const existingIndex = files.findIndex((f) => f.path === path);
      if (existingIndex !== -1) {
        setActiveIndex(existingIndex);
        return;
      }
      const result = (await window.spartan.call("open_file", { path })) as {
        doc_id: number;
        content: string;
      };
      setFiles((prev) => [
        ...prev,
        { path, docId: result.doc_id, content: result.content, dirty: false },
      ]);
      setActiveIndex(files.length);
    },
    [files]
  );

  const handleContentChange = useCallback((path: string, content: string, saved = false) => {
    setFiles((prev) =>
      prev.map((f) => (f.path === path ? { ...f, content, dirty: saved ? false : true } : f))
    );
  }, []);

  const closeFile = useCallback(
    (index: number) => {
      const file = files[index];
      window.spartan.call("close_file", { doc_id: file.docId }).catch(() => {});
      setFiles((prev) => prev.filter((_, i) => i !== index));
      setActiveIndex((prev) => Math.max(0, Math.min(prev, files.length - 2)));
      setDiagnosticsByDoc((prev) => {
        const next = { ...prev };
        delete next[file.docId];
        return next;
      });
      setBreakpointsByDoc((prev) => {
        const next = { ...prev };
        delete next[file.docId];
        return next;
      });
      setDapSessionByDoc((prev) => {
        const next = { ...prev };
        delete next[file.docId];
        return next;
      });
    },
    [files]
  );

  const activeFile = files[activeIndex] ?? null;

  const toggleBreakpoint = useCallback(
    (line: number) => {
      if (!activeFile) return;
      const docId = activeFile.docId;
      setBreakpointsByDoc((prev) => {
        const existing = prev[docId] ?? [];
        const next = existing.includes(line)
          ? existing.filter((l) => l !== line)
          : [...existing, line].sort((a, b) => a - b);
        return { ...prev, [docId]: next };
      });
    },
    [activeFile]
  );

  // Real launch (§132) -- always starts a fresh session for the active
  // file's own current breakpoint set, matching the reference wgpu
  // shell's own F5 convention (an already-finished session is treated
  // as gone, not resumable).
  const dapLaunch = useCallback(() => {
    if (!activeFile) return;
    const docId = activeFile.docId;
    const breakLines = breakpointsByDoc[docId] ?? [];
    setDapSessionByDoc((prev) => ({
      ...prev,
      [docId]: { sessionId: -1, status: "launching" },
    }));
    window.spartan
      .call("dap_launch", { doc_id: docId, break_lines: breakLines })
      .then((result) => {
        const { session_id } = result as { session_id: number };
        setDapSessionByDoc((prev) => ({
          ...prev,
          [docId]: { ...prev[docId], sessionId: session_id },
        }));
      })
      .catch((err: Error) => {
        setDapSessionByDoc((prev) => ({
          ...prev,
          [docId]: { sessionId: -1, status: "error", message: err.message },
        }));
      });
  }, [activeFile, breakpointsByDoc]);

  const dapSendCommand = useCallback(
    (method: string) => {
      if (!activeFile) return;
      const session = dapSessionByDoc[activeFile.docId];
      if (!session || session.sessionId < 0) return;
      window.spartan
        .call(method, { session_id: session.sessionId })
        .catch((err: Error) => console.error(`${method} failed:`, err));
    },
    [activeFile, dapSessionByDoc]
  );

  const dapStop = useCallback(() => {
    if (!activeFile) return;
    const docId = activeFile.docId;
    const session = dapSessionByDoc[docId];
    if (session && session.sessionId >= 0) {
      window.spartan.call("dap_disconnect", { session_id: session.sessionId }).catch(() => {});
    }
    setDapSessionByDoc((prev) => {
      const next = { ...prev };
      delete next[docId];
      return next;
    });
  }, [activeFile, dapSessionByDoc]);
  const screenLabel = NAV.flatMap((g) => g.items).find((i) => i.id === screen)?.label ?? screen;

  // Real §75.76 first-run onboarding gate -- deliberately blank (not the
  // main shell, not a spinner) during the brief real "checking" window
  // so there's no visible flash of the main UI before onboarding
  // decides whether to cover it.
  if (onboardingState === "checking") {
    return <div className="app-root" />;
  }
  if (onboardingState === "show") {
    return (
      <OnboardingScreen currentRoot={ROOT} onDone={() => setOnboardingState("done")} />
    );
  }

  return (
    <div className="app-root">
      <Sidebar active={screen} onSelect={setScreen} />
      <div className="main-column">
        {screen === "editor" ? (
          <>
            <div className="top-row">
              <TabBar
                files={files}
                activeIndex={activeIndex}
                onSelect={setActiveIndex}
                onClose={closeFile}
              />
            </div>
            <div className="editor-body">
              <div className="file-tree-panel">
                <div className="sidebar-toggle-row">
                  <button
                    className={`sidebar-toggle-btn ${sidebarView === "files" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("files")}
                  >
                    Files
                  </button>
                  <button
                    className={`sidebar-toggle-btn ${sidebarView === "git" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("git")}
                  >
                    Git
                  </button>
                  <button
                    className="sidebar-toggle-btn"
                    title="New Project"
                    onClick={() => setShowNewProjectWizard(true)}
                  >
                    + New
                  </button>
                </div>
                {sidebarView === "files" ? (
                  <FileTree root={ROOT} onOpenFile={openFile} />
                ) : (
                  <GitPanel root={ROOT} />
                )}
              </div>
              <div className="editor-and-debug">
                <DebugPanel
                  hasFile={activeFile !== null}
                  session={activeFile ? (dapSessionByDoc[activeFile.docId] ?? null) : null}
                  onLaunch={dapLaunch}
                  onContinue={() => dapSendCommand("dap_continue")}
                  onStepOver={() => dapSendCommand("dap_step_over")}
                  onStepInto={() => dapSendCommand("dap_step_into")}
                  onStop={dapStop}
                />
                <div className="content-area">
                  {activeFile ? (
                    <Editor
                      file={activeFile}
                      onContentChange={handleContentChange}
                      prefs={editorPrefs}
                      diagnostics={diagnosticsByDoc[activeFile.docId]}
                      breakpoints={breakpointsByDoc[activeFile.docId] ?? []}
                      onToggleBreakpoint={toggleBreakpoint}
                      stoppedLine={
                        dapSessionByDoc[activeFile.docId]?.status === "stopped"
                          ? (dapSessionByDoc[activeFile.docId]?.stopped?.frame?.line ?? null)
                          : null
                      }
                    />
                  ) : (
                    <div className="empty-state mono">Open a file from the sidebar to start editing.</div>
                  )}
                </div>
              </div>
            </div>
            <StatusBar
              fileCount={files.length}
              activePath={activeFile?.path ?? null}
              diagnostics={activeFile ? diagnosticsByDoc[activeFile.docId] : undefined}
            />
          </>
        ) : (
          <>
            <div className="top-row">
              <div className="screen-title mono">{screenLabel}</div>
            </div>
            <div className="content-area">
              {screen === "workflows" && <WorkflowsScreen />}
              {screen === "design" && (
                <DesignScreen activeFile={activeFile} onContentChange={handleContentChange} />
              )}
              {screen === "console" && <ConsoleScreen root={ROOT} />}
              {screen === "sessions" && <SessionsScreen root={ROOT} />}
              {screen === "settings" && <SettingsScreen />}
              {screen === "containers" && <DevContainersScreen root={ROOT} />}
              {screen !== "workflows" &&
                screen !== "design" &&
                screen !== "console" &&
                screen !== "sessions" &&
                screen !== "settings" &&
                screen !== "containers" && <Placeholder screen={screen} />}
            </div>
          </>
        )}
      </div>
      <LeoChatPanel projectRoot={ROOT} />
      {showNewProjectWizard && (
        <NewProjectWizard
          defaultParentDir={ROOT}
          onClose={() => setShowNewProjectWizard(false)}
          onCreated={(root) => window.spartan.openProject(root).then(() => {})}
        />
      )}
    </div>
  );
}
