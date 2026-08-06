import React, { useCallback, useEffect, useRef, useState } from "react";
import Sidebar from "./components/Sidebar";
import FileTree from "./components/FileTree";
import GitPanel from "./components/GitPanel";
import SearchPanel from "./components/SearchPanel";
import TabBar from "./components/TabBar";
import StatusBar, {
  type AndroidDetectResult,
  type AndroidBuildState,
  type AndroidDeviceInfo,
  type AndroidInstallState,
} from "./components/StatusBar";
import Editor, {
  type EditorPrefs,
  DEFAULT_EDITOR_PREFS,
  type OpenFile,
  type LspDiagnostic,
  type WorkspaceTextEdit,
  type BreakpointSpec,
} from "./components/Editor";
import type { UserSnippet } from "./snippets";
import DebugPanel, {
  type DapSessionState,
  type OutputEntry,
  type WatchEntry,
} from "./components/DebugPanel";
import LogcatPanel from "./components/LogcatPanel";
import Placeholder from "./components/Placeholder";
import WorkflowsScreen from "./components/WorkflowsScreen";
import ConsoleScreen from "./components/ConsoleScreen";
import SessionsScreen from "./components/SessionsScreen";
import SettingsScreen from "./components/SettingsScreen";
import DevContainersScreen from "./components/DevContainersScreen";
import ModelsScreen from "./components/ModelsScreen";
import LeoChatPanel from "./components/LeoChatPanel";
import NewProjectWizard from "./components/NewProjectWizard";
import UnsavedChangesModal from "./components/UnsavedChangesModal";
import OnboardingScreen from "./components/OnboardingScreen";
import DevicePreview from "./components/DevicePreview";
import { applyReduceMotion } from "./reduceMotion";
import { applyTheme, type ThemeName } from "./applyTheme";
import { applyFontFamily } from "./applyFontFamily";
import { NAV, type ScreenId } from "./nav";
import "./app.css";

const ROOT = new URLSearchParams(window.location.search).get("root") ?? "/";

/** Converts a real 0-indexed LSP line/character into a real absolute char
 * offset into `content` -- the same real math `Editor.tsx`'s own
 * `jumpToLocalPosition` already does locally, needed here at the `App.tsx`
 * level since a real rename's `WorkspaceEdit` may touch files that
 * component never mounted for. */
function lineCharToOffset(content: string, line: number, character: number): number {
  const lines = content.split("\n");
  let offset = 0;
  for (let i = 0; i < line && i < lines.length; i++) {
    offset += lines[i].length + 1; // +1 for the real newline this split consumed
  }
  offset += Math.min(character, lines[line]?.length ?? 0);
  return offset;
}

interface FindMatch {
  start: number;
  end: number;
}

/** Real, pure, plain-substring "find," ported verbatim from `Editor.tsx`'s
 * own identical wiring (task #223) -- see that file's own doc comment for
 * the full real reasoning (matches `search_project`'s own established
 * plain-substring, not regex, v1 scope, so "Replace in Files" below finds
 * exactly what the search results it operates on already found). */
function findAllMatches(content: string, query: string, caseSensitive: boolean): FindMatch[] {
  if (!query) return [];
  const haystack = caseSensitive ? content : content.toLowerCase();
  const needle = caseSensitive ? query : query.toLowerCase();
  const matches: FindMatch[] = [];
  let idx = haystack.indexOf(needle);
  while (idx !== -1) {
    matches.push({ start: idx, end: idx + needle.length });
    idx = haystack.indexOf(needle, idx + needle.length);
  }
  return matches;
}

/** Real "Replace All," ported verbatim from `Editor.tsx`'s own identical
 * wiring. */
function replaceAllMatches(content: string, matches: FindMatch[], replacement: string): string {
  let result = "";
  let cursor = 0;
  for (const m of matches) {
    result += content.slice(cursor, m.start) + replacement;
    cursor = m.end;
  }
  result += content.slice(cursor);
  return result;
}

export default function App(): React.ReactElement {
  const [files, setFiles] = useState<OpenFile[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  // Real unsaved-changes-on-close gate: the renderer's single source of
  // truth for "a close is waiting on a real user decision." `tab` is a
  // dirty file's × (TabBar); `app` is main.ts's `spartan:close-requested`
  // (window close / File > Quit / Cmd+Q). Only `onDiscard`/`onCancel`
  // resolve it -- never a silent close.
  const [pendingClose, setPendingClose] = useState<
    null | { kind: "tab"; index: number } | { kind: "app" }
  >(null);
  // Live `files` mirror for the `onCloseRequested` effect below, which is
  // registered once on mount and therefore can't close over fresh `files`;
  // this ref stays current so the handler always answers with the *real*
  // current dirty state, never a stale mount-time snapshot.
  const filesRef = useRef(files);
  filesRef.current = files;
  const [screen, setScreen] = useState<ScreenId>("editor");
  // Real Ctrl+G sidebar toggle (§75.30's own convention in the original
  // wgpu shell -- one left-rail region, not a second pane, shared between
  // the file tree and the real Source Control panel added in §75.65).
  const [sidebarView, setSidebarView] = useState<"files" | "git" | "search">("files");
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
  // Real go-to-definition cross-file jump target (task #164) -- set once a
  // real LSP definition result resolves to a *different* file than the one
  // currently open; `openFile` (below) opens/activates it, then this is
  // handed to `Editor.tsx` as `pendingJump`, filtered to only the file it
  // actually targets, so the newly-active file's own effect can land the
  // real cursor position once its content is available.
  const [pendingJump, setPendingJump] = useState<{
    path: string;
    line: number;
    character: number;
  } | null>(null);
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
  const [breakpointsByDoc, setBreakpointsByDoc] = useState<Record<number, BreakpointSpec[]>>({});
  const [dapSessionByDoc, setDapSessionByDoc] = useState<Record<number, DapSessionState>>({});
  // Real DAP output (§275) -- logpoints + the debuggee's own real stdout/
  // stderr, both relayed through the same `dap_output` event. Bounded per
  // doc so a chatty debuggee can't grow this unboundedly across a long
  // session, matching this codebase's own established bounded-log
  // precedent (e.g. `leo_session_history`'s cap).
  const [dapOutputByDoc, setDapOutputByDoc] = useState<Record<number, OutputEntry[]>>({});
  const MAX_DAP_OUTPUT_LINES = 500;
  // §75.98 program-path collection: an optional pre-built executable path
  // for the active file, supplied by the user in DebugPanel. Sent as
  // `program_path` on the next real `dap_launch`; only used when non-empty.
  const [programPath, setProgramPath] = useState("");
  // Real DAP watch/REPL expressions (§250) -- a debugger-wide list (not
  // per-doc), re-evaluated against the active session on every stop.
  // `watchResults` is keyed by expression; empty while not stopped.
  const [watchExpressions, setWatchExpressions] = useState<string[]>([]);
  const [watchResults, setWatchResults] = useState<
    Record<string, { value?: string; error?: string; pending?: boolean }>
  >({});
  const [androidInfo, setAndroidInfo] = useState<AndroidDetectResult | null>(null);
  // Real build state for task #144's "Build APK" action -- `idle` (never
  // triggered this session) is represented by `undefined`, not a real
  // phase, matching `StatusBar`'s own "no prop means no extra state" style
  // elsewhere.
  const [androidBuild, setAndroidBuild] = useState<AndroidBuildState | undefined>(undefined);
  // Real device-list + install state for task #148's next increment beyond
  // the build-only support above -- `androidDevices` is `undefined` until
  // the first `android_list_devices` call resolves (never fetched
  // proactively; only once a build is ready and the user clicks Install,
  // matching this component's own "don't call a real subprocess the user
  // hasn't asked for yet" convention).
  const [androidDevices, setAndroidDevices] = useState<AndroidDeviceInfo[] | undefined>(
    undefined
  );
  const [androidInstall, setAndroidInstall] = useState<AndroidInstallState | undefined>(
    undefined
  );
  // Real adb logcat streaming state (task #150) -- `logcatSessionId` is
  // the real session id `android_logcat_start` returns, `null` when no
  // session is currently live (never started, or already stopped/exited).
  const [logcatOpen, setLogcatOpen] = useState(false);
  const [logcatSessionId, setLogcatSessionId] = useState<number | null>(null);
  const [logcatLines, setLogcatLines] = useState<string[]>([]);

  // Real, one-shot on mount (ROOT is fixed for this window's lifetime, set
  // via the URL query param) -- android_detect has been real and tested
  // since §75.91 but had no UI caller anywhere in either shell until now.
  // A non-Android project (the common case) is a real, expected, silent
  // result, not an error.
  useEffect(() => {
    window.spartan
      .call("android_detect", { project_root: ROOT })
      .then((result) => setAndroidInfo(result as AndroidDetectResult))
      .catch(() => setAndroidInfo(null));
  }, []);

  // Real "ack now, event later" trigger for `android_build_apk` -- a real
  // Gradle `assembleDebug` build, which can easily run minutes on a cold
  // dependency cache, so this never blocks the click itself.
  const buildApk = useCallback(() => {
    setAndroidBuild({ phase: "building" });
    setAndroidInstall(undefined);
    setAndroidDevices(undefined);
    window.spartan.call("android_build_apk", { project_root: ROOT }).catch((e: Error) => {
      setAndroidBuild({ phase: "failed", error: e.message });
    });
  }, []);

  // Real "list, then install onto whichever real device is ready" flow
  // (task #148) -- lists first every click (not cached) since a real
  // device can be plugged/unplugged, or authorized, between clicks.
  // With zero or one ready device this is fully automatic; with more
  // than one, this deliberately picks the first ready one rather than
  // adding a device-picker UI in this first increment -- `adb -s` still
  // targets it correctly, and the tooltip lists every real device found
  // either way.
  const installApk = useCallback(() => {
    if (androidBuild?.phase !== "ready") return;
    const apkPath = androidBuild.apkPath;
    setAndroidInstall({ phase: "installing" });
    window.spartan
      .call("android_list_devices", {})
      .then((result) => {
        const devices = (result as { devices: AndroidDeviceInfo[] }).devices;
        setAndroidDevices(devices);
        const target = devices.find((d) => d.state === "device");
        if (!target && devices.length === 0) {
          throw new Error("no real device attached (adb devices -l reported none)");
        }
        return window.spartan.call("android_install_apk", {
          apk_path: apkPath,
          ...(target ? { serial: target.serial } : {}),
        });
      })
      .catch((e: Error) => {
        setAndroidInstall({ phase: "failed", error: e.message });
      });
  }, [androidBuild]);

  const toggleLogcat = useCallback(() => {
    setLogcatOpen((v) => !v);
  }, []);

  const startLogcat = useCallback(() => {
    setLogcatLines([]);
    window.spartan
      .call("android_logcat_start", {})
      .then((result) => {
        const { session_id } = result as { session_id: number };
        setLogcatSessionId(session_id);
      })
      .catch((e: Error) => {
        setLogcatLines([`error: ${e.message}`]);
      });
  }, []);

  const stopLogcat = useCallback(() => {
    if (logcatSessionId === null) return;
    window.spartan.call("android_logcat_stop", { session_id: logcatSessionId }).catch(() => {});
  }, [logcatSessionId]);

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
      } else if (event === "dap_output") {
        const { doc_id, category, text } = data as {
          doc_id: number;
          category: string;
          text: string;
        };
        setDapOutputByDoc((prev) => {
          const existing = prev[doc_id] ?? [];
          const next = [...existing, { category, text }];
          return {
            ...prev,
            [doc_id]: next.length > MAX_DAP_OUTPUT_LINES ? next.slice(-MAX_DAP_OUTPUT_LINES) : next,
          };
        });
      } else if (event === "android_build_progress") {
        const { line } = data as { line: string };
        setAndroidBuild((prev) =>
          prev?.phase === "building" ? { phase: "building", lastLine: line } : prev
        );
      } else if (event === "android_build_ready") {
        const { apk_path } = data as { apk_path: string };
        setAndroidBuild({ phase: "ready", apkPath: apk_path });
      } else if (event === "android_build_failed") {
        const { error } = data as { error: string };
        setAndroidBuild({ phase: "failed", error });
      } else if (event === "android_install_progress") {
        const { line } = data as { line: string };
        setAndroidInstall((prev) =>
          prev?.phase === "installing" ? { phase: "installing", lastLine: line } : prev
        );
      } else if (event === "android_install_ready") {
        setAndroidInstall({ phase: "ready" });
      } else if (event === "android_install_failed") {
        const { error } = data as { error: string };
        setAndroidInstall({ phase: "failed", error });
      } else if (event === "android_logcat_output") {
        // Real, deliberate v1 simplification: this UI only ever starts
        // one real logcat session at a time, so every real output event
        // is appended without matching `session_id` against a ref --
        // correct as long as that stays true, named rather than silently
        // assumed.
        const { line } = data as { session_id: number; line: string };
        setLogcatLines((prev) => [...prev, line]);
      } else if (event === "android_logcat_exit") {
        setLogcatSessionId(null);
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
            format_on_save?: boolean;
          };
          user_snippets?: UserSnippet[];
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
            formatOnSave: s.editor.format_on_save ?? DEFAULT_EDITOR_PREFS.formatOnSave,
            userSnippets: s.user_snippets ?? [],
          });
        }
      })
      .catch(() => setOnboardingState("done"));
  }, []);

  // Real user-snippets freshness: the mount effect above runs once, so
  // settings changed while the Settings screen was showing (a new or
  // edited user snippet above all) would otherwise reach the editor only
  // on a full app restart. Re-fetch the real settings every time the user
  // navigates back to the editor screen (which unmounts and remounts the
  // `Editor`, so the fresh `prefs` are picked up immediately). The
  // initial mount is skipped -- the effect above already covered it.
  const firstScreenRender = useRef(true);
  useEffect(() => {
    if (firstScreenRender.current) {
      firstScreenRender.current = false;
      return;
    }
    if (screen !== "editor") return;
    window.spartan
      .call("settings_get", {})
      .then((result) => {
        const s = result as {
          editor?: {
            font_size?: number;
            tab_size?: number;
            word_wrap?: boolean;
            font_family?: string | null;
            format_on_save?: boolean;
          };
          user_snippets?: UserSnippet[];
        };
        if (s.editor) {
          setEditorPrefs({
            fontSize: s.editor.font_size ?? DEFAULT_EDITOR_PREFS.fontSize,
            tabSize: s.editor.tab_size ?? DEFAULT_EDITOR_PREFS.tabSize,
            wordWrap: s.editor.word_wrap ?? DEFAULT_EDITOR_PREFS.wordWrap,
            formatOnSave: s.editor.format_on_save ?? DEFAULT_EDITOR_PREFS.formatOnSave,
            userSnippets: s.user_snippets ?? [],
          });
        }
      })
      .catch(() => {});
  }, [screen]);

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

  /** Real cross-file go-to-definition landing: opens (or activates, via
   * `openFile`'s own existing dedup) the real target file, then hands the
   * real jump position down once that file is active -- `Editor.tsx`'s own
   * `pendingJump` effect applies it and reports back via `onJumpApplied`. */
  const handleJumpToDefinition = useCallback(
    async (path: string, line: number, character: number) => {
      try {
        await openFile(path);
        setPendingJump({ path, line, character });
      } catch (err) {
        console.error("go-to-definition: failed to open the real target file:", err);
      }
    },
    [openFile]
  );

  const handleContentChange = useCallback((path: string, content: string, saved = false) => {
    setFiles((prev) =>
      prev.map((f) => (f.path === path ? { ...f, content, dirty: saved ? false : true } : f))
    );
  }, []);

  /** Real F2 rename-symbol apply -- for each real file a resolved
   * `WorkspaceEdit` touches, opens it (or reuses an already-open tab, via
   * the same `files`-scanning `openFile` itself already does) and applies
   * every real edit through the existing, already-real `edit` IPC method.
   * Edits within one file are applied in descending-start-offset order,
   * computed once from that file's own real original content (`content`,
   * captured before any edit in this call touches it) -- since a real
   * `WorkspaceEdit`'s own edits never overlap (the LSP spec's own
   * guarantee) and are applied highest-offset-first, every not-yet-applied
   * edit's own original offset stays valid throughout, so there's no need
   * to re-derive offsets against a progressively-mutated buffer. Resolves
   * to the real number of files touched, so `Editor.tsx`'s own rename UI
   * can report a real result instead of assuming success. */
  const applyRename = useCallback(
    async (changes: Record<string, WorkspaceTextEdit[]>): Promise<number> => {
      let touchedCount = 0;
      for (const [path, edits] of Object.entries(changes)) {
        let docId: number;
        let content: string;
        const existing = files.find((f) => f.path === path);
        if (existing) {
          docId = existing.docId;
          content = existing.content;
        } else {
          const result = (await window.spartan.call("open_file", { path })) as {
            doc_id: number;
            content: string;
          };
          docId = result.doc_id;
          content = result.content;
          setFiles((prev) => [...prev, { path, docId, content, dirty: false }]);
        }

        const withOffsets = edits
          .map((e) => ({
            edit: e,
            startOffset: lineCharToOffset(content, e.startLine, e.startCharacter),
            endOffset: lineCharToOffset(content, e.endLine, e.endCharacter),
          }))
          .sort((a, b) => b.startOffset - a.startOffset);

        let working = content;
        for (const { edit, startOffset, endOffset } of withOffsets) {
          await window.spartan.call("edit", {
            doc_id: docId,
            start_char: startOffset,
            end_char: endOffset,
            text: edit.newText,
          });
          working = working.slice(0, startOffset) + edit.newText + working.slice(endOffset);
        }
        handleContentChange(path, working);
        touchedCount++;
      }
      return touchedCount;
    },
    [files, handleContentChange]
  );

  /** Real "Replace in Files" -- the bulk-replace half of the already-real
   * "Find in Files" panel (tasks #190-192), closing the gap between it and
   * the in-buffer "Find & Replace" that only ever touches the one
   * currently open file (task #223). Deliberately reuses `applyRename`'s
   * own real open-or-reuse-then-`edit` shape rather than a second, parallel
   * multi-file-apply implementation -- both are "given a set of real
   * cross-file text changes, get every affected file open and correctly
   * updated," the only real difference is where the change list comes from
   * (an LSP `WorkspaceEdit` there, a real client-side substring recompute
   * here). A real, deliberate correctness property, not incidental: this
   * recomputes `findAllMatches` against each file's own *current* content
   * (freshly opened from disk, or the live buffer of an already-open tab)
   * at replace time, never against the search panel's own possibly-stale
   * preview text -- so a file edited since the last search still gets
   * exactly the real matches it actually has, never a phantom or
   * missed one. Applied as a single whole-file replace per file (matching
   * `triggerFormatDocument`'s own established "one edit, one undo
   * checkpoint" convention for a document-wide programmatic change),
   * not per-match range edits. */
  const applyReplaceInFiles = useCallback(
    async (
      matches: { path: string; line: number; text: string }[],
      query: string,
      replacement: string
    ): Promise<{ filesChanged: number; totalReplacements: number }> => {
      const relPaths = Array.from(new Set(matches.map((m) => m.path)));
      let filesChanged = 0;
      let totalReplacements = 0;
      for (const relPath of relPaths) {
        const path = `${ROOT.replace(/\/+$/, "")}/${relPath}`;
        let docId: number;
        let content: string;
        const existing = files.find((f) => f.path === path);
        if (existing) {
          docId = existing.docId;
          content = existing.content;
        } else {
          const result = (await window.spartan.call("open_file", { path })) as {
            doc_id: number;
            content: string;
          };
          docId = result.doc_id;
          content = result.content;
          setFiles((prev) => [...prev, { path, docId, content, dirty: false }]);
        }

        const found = findAllMatches(content, query, true);
        if (found.length === 0) continue;
        const next = replaceAllMatches(content, found, replacement);
        const oldLength = [...content].length;
        await window.spartan.call("edit", {
          doc_id: docId,
          start_char: 0,
          end_char: oldLength,
          text: next,
        });
        handleContentChange(path, next);
        filesChanged++;
        totalReplacements += found.length;
      }
      return { filesChanged, totalReplacements };
    },
    [files, handleContentChange]
  );

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

  /** Real unsaved-changes-on-close gate for a single tab: a dirty file's
   * × raises the confirmation modal instead of silently discarding; a
   * clean file closes immediately, exactly as before. `closeFile` itself
   * stays the unconditional real-close implementation, reused unchanged by
   * both the modal's Discard path and this gate's clean-file fast path. */
  const requestCloseFile = useCallback(
    (index: number) => {
      const file = files[index];
      if (file?.dirty) {
        setPendingClose({ kind: "tab", index });
      } else {
        closeFile(index);
      }
    },
    [files, closeFile]
  );

  /** Real resolution of the pending close: Discard closes the dirty tab
   * (or confirms the whole-app close to main.ts), then clears the pending
   * state; Cancel just clears it, leaving the tab/app untouched.
   * Deliberately reads `pendingClose` from the closure (not a functional
   * `setPendingClose` updater), so the side-effecting `closeFile` runs in
   * the event handler itself, never inside a state updater -- which React
   * StrictMode dev can invoke twice. */
  const handleDiscardPendingClose = useCallback(() => {
    if (pendingClose?.kind === "tab") closeFile(pendingClose.index);
    else if (pendingClose?.kind === "app") window.spartan.confirmClose();
    setPendingClose(null);
  }, [pendingClose, closeFile]);

  // Real whole-app close gate (main.ts's `win.on("close", ...)` prevents
  // every window close and asks us, the only place that knows the real
  // per-tab dirty state): answer immediately when nothing is dirty, prompt
  // when something is. Registered once on mount -- `filesRef` is what keeps
  // the handler's dirty check real and current.
  useEffect(() => {
    return window.spartan.onCloseRequested(() => {
      if (filesRef.current.some((f) => f.dirty)) {
        setPendingClose({ kind: "app" });
      } else {
        window.spartan.confirmClose();
      }
    });
  }, []);

  const toggleBreakpoint = useCallback(
    (line: number) => {
      if (!activeFile) return;
      const docId = activeFile.docId;
      setBreakpointsByDoc((prev) => {
        const existing = prev[docId] ?? [];
        const next = existing.some((b) => b.line === line)
          ? existing.filter((b) => b.line !== line)
          : [...existing, { line }].sort((a, b) => a.line - b.line);
        return { ...prev, [docId]: next };
      });
    },
    [activeFile]
  );

  // Real right-click condition/logpoint edit -- sets the given line's
  // condition/log message (creating a breakpoint there if none exists);
  // empty strings for both clear it back to a plain breakpoint.
  const editBreakpoint = useCallback(
    (line: number, condition: string, logMessage: string) => {
      if (!activeFile) return;
      const docId = activeFile.docId;
      setBreakpointsByDoc((prev) => {
        const existing = prev[docId] ?? [];
        const spec: BreakpointSpec = { line };
        if (condition) spec.condition = condition;
        if (logMessage) spec.logMessage = logMessage;
        const others = existing.filter((b) => b.line !== line);
        const next = [...others, spec].sort((a, b) => a.line - b.line);
        return { ...prev, [docId]: next };
      });
    },
    [activeFile]
  );

  // Real rope-anchored breakpoint shifting (closes the §75.8-named
  // "line-number only" gap) -- `Editor.tsx` already computed the full,
  // correctly shifted/dropped array via `shiftBreakpointsForEdit`; this
  // just commits it to the real owned state, the same division of
  // responsibility `toggleBreakpoint`/`editBreakpoint` already use.
  const handleBreakpointsShift = useCallback(
    (next: BreakpointSpec[]) => {
      if (!activeFile) return;
      const docId = activeFile.docId;
      setBreakpointsByDoc((prev) => ({ ...prev, [docId]: next }));
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
    const breakpoints = breakpointsByDoc[docId] ?? [];
    setDapSessionByDoc((prev) => ({
      ...prev,
      [docId]: { sessionId: -1, status: "launching" },
    }));
    // A fresh launch starts a genuinely new debuggee -- stale output from
    // a prior run must not linger under it.
    setDapOutputByDoc((prev) => ({ ...prev, [docId]: [] }));
    window.spartan
      .call("dap_launch", {
        doc_id: docId,
        breakpoints: breakpoints.map((b) => ({
          line: b.line,
          condition: b.condition,
          logMessage: b.logMessage,
        })),
        ...(programPath.trim() ? { program_path: programPath.trim() } : {}),
      })
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
  }, [activeFile, breakpointsByDoc, programPath]);

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

  // Real watch/REPL evaluation (§250) -- evaluates one expression against a
  // stopped session and records its value or a real error.
  const evaluateWatch = useCallback((sessionId: number, expression: string) => {
    setWatchResults((prev) => ({ ...prev, [expression]: { pending: true } }));
    window.spartan
      .call("dap_evaluate", { session_id: sessionId, expression })
      .then((res) => {
        const { result } = res as { result: string };
        setWatchResults((prev) => ({ ...prev, [expression]: { value: result } }));
      })
      .catch((err: Error) => {
        setWatchResults((prev) => ({ ...prev, [expression]: { error: err.message } }));
      });
  }, []);

  const addWatch = useCallback(
    (expression: string) => {
      setWatchExpressions((prev) => (prev.includes(expression) ? prev : [...prev, expression]));
      // Evaluate immediately if a session is currently stopped.
      if (activeFile) {
        const session = dapSessionByDoc[activeFile.docId];
        if (session && session.sessionId >= 0 && session.status === "stopped") {
          evaluateWatch(session.sessionId, expression);
        }
      }
    },
    [activeFile, dapSessionByDoc, evaluateWatch]
  );

  const removeWatch = useCallback((expression: string) => {
    setWatchExpressions((prev) => prev.filter((e) => e !== expression));
    setWatchResults((prev) => {
      const next = { ...prev };
      delete next[expression];
      return next;
    });
  }, []);

  // Real DAP setVariable (task #276) -- edits a variable's live value in
  // the current top scope. The backend already queues a fresh
  // `dap_stopped` event on success (with reason "variable_edit"), which
  // flows through the exact same event handler above and updates both
  // the Variables panel and (via the effect below) every open Watch --
  // no separate refresh call needed here.
  const setVariable = useCallback(
    (name: string, value: string) => {
      if (!activeFile) return;
      const session = dapSessionByDoc[activeFile.docId];
      if (!session || session.sessionId < 0) return;
      window.spartan
        .call("dap_set_variable", { session_id: session.sessionId, name, value })
        .catch((err: Error) => console.error("dap_set_variable failed:", err));
    },
    [activeFile, dapSessionByDoc]
  );

  const activeSession = activeFile ? (dapSessionByDoc[activeFile.docId] ?? null) : null;
  const activeSessionId = activeSession?.sessionId ?? -1;
  const activeSessionStatus = activeSession?.status;
  // A fresh `stopped` object arrives on every real stop event (even one that
  // lands on the same line, e.g. a loop breakpoint) -- keying the effect on
  // its reference re-evaluates watches against each new frame's real values.
  const activeStopped = activeSession?.stopped;
  useEffect(() => {
    if (activeSessionStatus === "stopped" && activeSessionId >= 0) {
      watchExpressions.forEach((expr) => evaluateWatch(activeSessionId, expr));
    } else {
      // Not stopped -- prior results are stale; clear them rather than show
      // values that no longer reflect any live frame.
      setWatchResults((prev) => (Object.keys(prev).length ? {} : prev));
    }
    // watchExpressions is intentionally excluded: a newly-added watch is
    // evaluated eagerly in `addWatch`; this effect only re-evaluates the
    // whole set on a real stop transition.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId, activeSessionStatus, activeStopped, evaluateWatch]);

  const watchEntries: WatchEntry[] = watchExpressions.map((expression) => ({
    expression,
    ...watchResults[expression],
  }));
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
                onClose={requestCloseFile}
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
                    className={`sidebar-toggle-btn ${sidebarView === "search" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("search")}
                  >
                    Search
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
                ) : sidebarView === "git" ? (
                  <GitPanel root={ROOT} />
                ) : (
                  <SearchPanel
                    root={ROOT}
                    onOpenResult={(absPath, line) => handleJumpToDefinition(absPath, line, 0)}
                    onReplaceAll={applyReplaceInFiles}
                  />
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
                  watches={watchEntries}
                  onAddWatch={addWatch}
                  onRemoveWatch={removeWatch}
                  onSetVariable={setVariable}
                  outputLog={activeFile ? dapOutputByDoc[activeFile.docId] : undefined}
                  programPath={programPath}
                  onProgramPathChange={setProgramPath}
                />
                <LogcatPanel
                  visible={logcatOpen}
                  running={logcatSessionId !== null}
                  lines={logcatLines}
                  onStart={startLogcat}
                  onStop={stopLogcat}
                  onClose={() => setLogcatOpen(false)}
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
                      onEditBreakpoint={editBreakpoint}
                      onBreakpointsShift={handleBreakpointsShift}
                      stoppedLine={
                        dapSessionByDoc[activeFile.docId]?.status === "stopped"
                          ? (dapSessionByDoc[activeFile.docId]?.stopped?.frame?.line ?? null)
                          : null
                      }
                      onJumpToDefinition={handleJumpToDefinition}
                      pendingJump={
                        pendingJump && pendingJump.path === activeFile.path ? pendingJump : null
                      }
                      onJumpApplied={() => setPendingJump(null)}
                      onApplyRename={applyRename}
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
              androidInfo={androidInfo}
              androidBuild={androidBuild}
              onBuildApk={buildApk}
              androidDevices={androidDevices}
              androidInstall={androidInstall}
              onInstallApk={installApk}
              logcatOpen={logcatOpen}
              logcatRunning={logcatSessionId !== null}
              onToggleLogcat={toggleLogcat}
            />
          </>
        ) : (
          <>
            <div className="top-row">
              <div className="screen-title mono">{screenLabel}</div>
            </div>
            <div className="content-area">
              {screen === "workflows" && <WorkflowsScreen />}
              {screen === "console" && <ConsoleScreen root={ROOT} />}
              {screen === "sessions" && <SessionsScreen root={ROOT} />}
              {screen === "settings" && <SettingsScreen />}
              {screen === "containers" && <DevContainersScreen root={ROOT} />}
              {screen === "models" && <ModelsScreen />}
              {screen === "device-preview" && <DevicePreview />}
              {screen !== "workflows" &&
                screen !== "console" &&
                screen !== "sessions" &&
                screen !== "settings" &&
                screen !== "containers" &&
                screen !== "models" &&
                screen !== "device-preview" && <Placeholder screen={screen} />}
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
      {pendingClose && (
        <UnsavedChangesModal
          fileNames={
            pendingClose.kind === "tab"
              ? files[pendingClose.index]
                ? [files[pendingClose.index].path]
                : []
              : files.filter((f) => f.dirty).map((f) => f.path)
          }
          onDiscard={handleDiscardPendingClose}
          onCancel={() => setPendingClose(null)}
        />
      )}
    </div>
  );
}
