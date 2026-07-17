import React, { useCallback, useEffect, useState } from "react";
import FileTree from "./components/FileTree";
import GitPanel from "./components/GitPanel";
import ModelsPanel from "./components/ModelsPanel";
import BackendFileTree from "./components/BackendFileTree";
import Editor, { type OpenFile } from "./components/Editor";
import BackendEditor, { type BackendOpenFile, type LspDiagnostic } from "./components/BackendEditor";
import DebugPanel, { type DapSessionState } from "./components/DebugPanel";
import { ensureBufferWasmInit, Document as WasmDocument } from "./buffer";
import { isFileSystemAccessSupported, pickProjectDirectory, readFileText } from "./fsAccess";
import { applyTheme, type ThemeName } from "./applyTheme";
import { applyFontFamily } from "./applyFontFamily";
import { BackendClient } from "./backendClient";

type ActiveContent =
  | { kind: "local"; file: OpenFile }
  | { kind: "backend"; file: BackendOpenFile }
  | null;

type SidebarView = "files" | "git" | "backend" | "models";

type BackendStatus = "connecting" | "connected" | "client-only";

/** Real shape of `spartan-backend`'s `android_detect` method -- byte-
 * identical to desktop/'s own `StatusBar.tsx` copy (task #142/#146), not
 * shared code since the two shells don't share a components package. */
interface AndroidDetectResult {
  isAndroidProject: boolean;
  sdkRoot: string | null;
  adbPath: string | null;
  emulatorPath: string | null;
  sdkmanagerPath: string | null;
  avdmanagerPath: string | null;
  gradlePath: string | null;
  gradleVersion: string | null;
}

/** Real client-side state for the "Build APK" action -- byte-identical to
 * desktop/'s own `StatusBar.tsx` copy (task #144/#146). */
type AndroidBuildState =
  | { phase: "idle" }
  | { phase: "building"; lastLine?: string }
  | { phase: "ready"; apkPath: string }
  | { phase: "failed"; error: string };

/** Real shape of one entry from `android_list_devices`'s own real
 * `adb devices -l` parse (task #148) -- byte-identical to desktop/'s own
 * `StatusBar.tsx` copy. */
interface AndroidDeviceInfo {
  serial: string;
  state: string;
  model: string | null;
  product: string | null;
}

/** Real client-side state for the "Install APK" action (task #148) --
 * byte-identical to desktop/'s own `StatusBar.tsx` copy. */
type AndroidInstallState =
  | { phase: "idle" }
  | { phase: "installing"; lastLine?: string }
  | { phase: "ready" }
  | { phase: "failed"; error: string };

// Real §75.93 theme/font persistence -- this app has no `spartan-backend`
// settings store to round-trip through (§75.89's own named scope: no
// LSP/DAP/Leo/git connectivity yet), so a real, local `localStorage` key
// stands in, the same pattern `desktop/`'s own Leo voice-output toggle
// already established (§75.71) for a pure renderer preference.
const THEME_STORAGE_KEY = "spartan.theme";
const FONT_STORAGE_KEY = "spartan.fontFamily";

/**
 * Real top-level shell for the web app's first real increment (task #81,
 * §75.89) -- the pure client-side half of the hybrid architecture
 * (§75.85): real local folder/file access via the File System Access
 * API, real editing backed by the real `spartan-buffer` engine compiled
 * to WASM, real save-to-disk, real (single-step) undo.
 *
 * **Real, deliberately out-of-scope in this first increment, named
 * honestly rather than silently missing**: Leo is still not wired to
 * anything here (DAP now is -- see below). `spartan-backend`'s real
 * WebSocket transport (§75.88) exists and is real, tested, production
 * code; a later increment answered the token-delivery design question
 * that transport's own doc comment explicitly left open (how a browser
 * tab legitimately learns the per-process token and the correct origin --
 * the `/__spartan/session` same-origin handoff), a further increment used
 * that same handoff to advertise the devserver's own real project root so
 * **git is real and wired** (`GitPanel`), a further increment gave this
 * app a real **backend-mode editing path** (`BackendFileTree`/
 * `BackendEditor`, routing file open/edit/undo/redo/save through
 * `BackendClient` instead of File System Access + WASM) purely so
 * `spartan-backend`'s own real LSP diagnostics wiring -- which needs a
 * real `doc_id` the WASM path has no equivalent for -- has something to
 * attach to here, the same way it already does in `desktop/`, and a
 * further increment (task #133) extended that same backend-mode path with
 * **real DAP debugging** (`DebugPanel`, click-to-toggle breakpoints in
 * `BackendEditor`'s own gutter) -- `spartan-devserver` already falls every
 * `dap_*` method through to `spartan_backend::handle_request` unchanged,
 * so this needed zero backend/protocol changes, purely a UI addition. The
 * two editing paths are independent and both real: File System Access +
 * WASM (`FileTree`/`Editor`) works with no backend at all;
 * `BackendFileTree`/`BackendEditor` only appear once a devserver is
 * connected with a known project root, and operate on that root, not
 * necessarily the File System Access folder. Multi-file tabs are also not
 * built yet -- only one file open at a time (whichever kind was opened
 * most recently), the same real, narrow first-increment scoping this
 * project's own history already applies elsewhere (e.g. `gui-builder`'s
 * own real v1 cuts, §75.38).
 */
export default function App(): React.ReactElement {
  const [root, setRoot] = useState<FileSystemDirectoryHandle | null>(null);
  const [activeContent, setActiveContent] = useState<ActiveContent>(null);
  const [error, setError] = useState<string | null>(null);
  const [wasmReady, setWasmReady] = useState(false);
  const [theme, setTheme] = useState<ThemeName>(
    () => (localStorage.getItem(THEME_STORAGE_KEY) as ThemeName | null) ?? "SpartanDark"
  );
  const [fontFamily, setFontFamily] = useState<string>(
    () => localStorage.getItem(FONT_STORAGE_KEY) ?? ""
  );
  const [backendStatus, setBackendStatus] = useState<BackendStatus>("connecting");
  const [backendClient, setBackendClient] = useState<BackendClient | null>(null);
  const [sidebarView, setSidebarView] = useState<SidebarView>("files");
  // Real, live LSP diagnostics, keyed by doc_id (only ever populated for
  // backend-mode files -- the WASM path has no doc_id for this to key
  // off of). Kept across a sidebar-view switch so re-opening the same
  // backend file doesn't lose its already-known diagnostics.
  const [diagnosticsByDoc, setDiagnosticsByDoc] = useState<Record<number, LspDiagnostic[]>>({});
  // Real DAP state (task #133, extending task #132's desktop/ wiring to
  // this app), both keyed by `doc_id` the same way `diagnosticsByDoc`
  // already is -- breakpoints are 1-indexed line numbers matching the
  // gutter's own display and the real `dap_launch` `break_lines` param
  // directly; a session entry exists only while a debug session for that
  // file is live or has just finished (exited/errored), so the toolbar
  // can show its final state before the user dismisses it via Stop or
  // relaunches.
  const [breakpointsByDoc, setBreakpointsByDoc] = useState<Record<number, number[]>>({});
  const [dapSessionByDoc, setDapSessionByDoc] = useState<Record<number, DapSessionState>>({});
  // Real android_detect/android_build_apk state (task #146), the web/
  // sibling of desktop/'s own StatusBar wiring (tasks #142/#144) -- these
  // are real spartan-backend methods reached generically through
  // BackendClient, with no method allowlist to extend the way desktop/'s
  // preload.ts needed.
  const [androidInfo, setAndroidInfo] = useState<AndroidDetectResult | null>(null);
  const [androidBuild, setAndroidBuild] = useState<AndroidBuildState | undefined>(undefined);
  // Real device-list + install state (task #148/#149), the web/ sibling of
  // desktop/'s own `StatusBar.tsx` wiring -- fetched fresh on every
  // Install click, never cached (a real device can be plugged/unplugged
  // between clicks).
  const [androidDevices, setAndroidDevices] = useState<AndroidDeviceInfo[] | undefined>(
    undefined
  );
  const [androidInstall, setAndroidInstall] = useState<AndroidInstallState | undefined>(
    undefined
  );

  // Optional backend upgrade: when this page is served by a real
  // spartan-devserver (§75.88 + the /__spartan/session handoff), connect to
  // it. When it isn't (a Vite dev server, plain static hosting), the fetch
  // 404s / rejects and the app stays in its existing pure-client mode --
  // no error surfaced to the user, connectivity is genuinely optional.
  useEffect(() => {
    let client: BackendClient | null = null;
    let cancelled = false;
    BackendClient.connect()
      .then((c) => {
        if (cancelled) {
          c.close();
          return;
        }
        client = c;
        setBackendClient(c);
        setBackendStatus("connected");
      })
      .catch(() => {
        if (!cancelled) setBackendStatus("client-only");
      });
    return () => {
      cancelled = true;
      client?.close();
      setBackendClient(null);
    };
  }, []);

  // Real, one-shot android_detect once a real project root is known -- the
  // web/ sibling of desktop/'s own on-mount call (task #142), just keyed
  // off the real devserver's own resolved project root instead of a fixed
  // URL query param. A non-Android project (the common case) is a real,
  // expected, silent result, not an error.
  useEffect(() => {
    const projectRoot = backendClient?.projectRoot;
    if (!backendClient || !projectRoot) {
      setAndroidInfo(null);
      return;
    }
    backendClient
      .call("android_detect", { project_root: projectRoot })
      .then((result) => setAndroidInfo(result as AndroidDetectResult))
      .catch(() => setAndroidInfo(null));
  }, [backendClient]);

  // Real "ack now, event later" trigger for android_build_apk -- mirrors
  // desktop/'s own `buildApk` callback exactly.
  const buildApk = useCallback(() => {
    const projectRoot = backendClient?.projectRoot;
    if (!backendClient || !projectRoot) return;
    setAndroidBuild({ phase: "building" });
    setAndroidInstall(undefined);
    setAndroidDevices(undefined);
    backendClient.call("android_build_apk", { project_root: projectRoot }).catch((e: Error) => {
      setAndroidBuild({ phase: "failed", error: e.message });
    });
  }, [backendClient]);

  // Real "list, then install onto whichever real device is ready" flow
  // (task #148/#149) -- mirrors desktop/'s own `installApk` callback
  // exactly, just reached through `BackendClient` instead of
  // `window.spartan`.
  const installApk = useCallback(() => {
    if (!backendClient || androidBuild?.phase !== "ready") return;
    const apkPath = androidBuild.apkPath;
    setAndroidInstall({ phase: "installing" });
    backendClient
      .call("android_list_devices", {})
      .then((result) => {
        const devices = (result as { devices: AndroidDeviceInfo[] }).devices;
        setAndroidDevices(devices);
        const target = devices.find((d) => d.state === "device");
        if (!target && devices.length === 0) {
          throw new Error("no real device attached (adb devices -l reported none)");
        }
        return backendClient.call("android_install_apk", {
          apk_path: apkPath,
          ...(target ? { serial: target.serial } : {}),
        });
      })
      .catch((e: Error) => {
        setAndroidInstall({ phase: "failed", error: e.message });
      });
  }, [backendClient, androidBuild]);

  // Real, live LSP diagnostics stream -- the same real lsp_diagnostics/
  // lsp_error events desktop/'s App.tsx subscribes to via window.spartan.
  // onEvent, here via BackendClient.onEvent's single-argument
  // {event, data} shape instead.
  useEffect(() => {
    if (!backendClient) return;
    return backendClient.onEvent((e) => {
      if (e.event === "lsp_diagnostics") {
        const { doc_id, diagnostics } = e.data as { doc_id: number; diagnostics: LspDiagnostic[] };
        setDiagnosticsByDoc((prev) => ({ ...prev, [doc_id]: diagnostics }));
      } else if (e.event === "lsp_error") {
        console.warn("lsp_error:", e.data);
      } else if (e.event === "dap_stopped") {
        const { doc_id, stopped } = e.data as {
          doc_id: number;
          stopped: DapSessionState["stopped"];
        };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return { ...prev, [doc_id]: { ...existing, status: "stopped", stopped } };
        });
      } else if (e.event === "dap_exited") {
        const { doc_id } = e.data as { doc_id: number };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return { ...prev, [doc_id]: { ...existing, status: "exited" } };
        });
      } else if (e.event === "dap_error") {
        const { doc_id, message } = e.data as { doc_id: number; message: string };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return { ...prev, [doc_id]: { ...existing, status: "error", message } };
        });
      } else if (e.event === "dap_build_failed") {
        const { doc_id, diagnostics } = e.data as { doc_id: number; diagnostics: string[] };
        setDapSessionByDoc((prev) => {
          const existing = prev[doc_id];
          if (!existing) return prev;
          return {
            ...prev,
            [doc_id]: { ...existing, status: "build_failed", message: diagnostics.join("\n") },
          };
        });
      } else if (e.event === "android_build_progress") {
        const { line } = e.data as { line: string };
        setAndroidBuild((prev) =>
          prev?.phase === "building" ? { phase: "building", lastLine: line } : prev
        );
      } else if (e.event === "android_build_ready") {
        const { apk_path } = e.data as { apk_path: string };
        setAndroidBuild({ phase: "ready", apkPath: apk_path });
      } else if (e.event === "android_build_failed") {
        const { error } = e.data as { error: string };
        setAndroidBuild({ phase: "failed", error });
      } else if (e.event === "android_install_progress") {
        const { line } = e.data as { line: string };
        setAndroidInstall((prev) =>
          prev?.phase === "installing" ? { phase: "installing", lastLine: line } : prev
        );
      } else if (e.event === "android_install_ready") {
        setAndroidInstall({ phase: "ready" });
      } else if (e.event === "android_install_failed") {
        const { error } = e.data as { error: string };
        setAndroidInstall({ phase: "failed", error });
      }
    });
  }, [backendClient]);

  // A real, connected devserver with a real, resolved project root is what
  // makes the Git panel *and* backend-mode editing usable -- the File
  // System Access API never gives this app an OS path for whatever folder
  // `root` (above) points at, so both operate on the devserver's *own*
  // launch directory instead (see `GitPanel`'s/`BackendFileTree`'s and
  // `backendClient.ts`'s own doc comments).
  const backendReady = backendStatus === "connected" && !!backendClient?.projectRoot;
  // model_status/litellm_proxy_*/hf_* are real devserver methods that need
  // no project root at all (unlike git/backend-mode editing above), so this
  // tab is available as soon as any devserver connection is live.
  const backendConnected = backendStatus === "connected" && !!backendClient;
  const availableSidebarViews: SidebarView[] = [
    ...(root ? (["files"] as const) : []),
    ...(backendReady ? (["git", "backend"] as const) : []),
    ...(backendConnected ? (["models"] as const) : []),
  ];
  const activeSidebarView: SidebarView = availableSidebarViews.includes(sidebarView)
    ? sidebarView
    : (availableSidebarViews[0] ?? "files");

  // Applied on mount and every real change, matching `desktop/`'s own
  // startup-apply-then-live-apply pattern (`App.tsx`/`SettingsScreen.tsx`).
  useEffect(() => {
    applyTheme(theme);
    localStorage.setItem(THEME_STORAGE_KEY, theme);
  }, [theme]);

  useEffect(() => {
    applyFontFamily(fontFamily);
    localStorage.setItem(FONT_STORAGE_KEY, fontFamily);
  }, [fontFamily]);

  const openFolder = useCallback(async () => {
    setError(null);
    try {
      await ensureBufferWasmInit();
      setWasmReady(true);
      const dir = await pickProjectDirectory();
      setRoot(dir);
    } catch (e) {
      // A real, expected case, not a bug: the user cancelling the real
      // native folder picker also rejects this promise.
      if (e instanceof DOMException && e.name === "AbortError") return;
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const openFile = useCallback(
    async (handle: FileSystemFileHandle, path: string) => {
      setError(null);
      try {
        if (!wasmReady) {
          await ensureBufferWasmInit();
          setWasmReady(true);
        }
        const content = await readFileText(handle);
        const doc = new WasmDocument(content);
        setActiveContent({ kind: "local", file: { path, handle, doc, content, dirty: false } });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [wasmReady]
  );

  const openBackendFile = useCallback(
    async (path: string) => {
      if (!backendClient) return;
      setError(null);
      try {
        const result = (await backendClient.call("open_file", { path })) as {
          doc_id: number;
          content: string;
        };
        setActiveContent({
          kind: "backend",
          file: { path, docId: result.doc_id, content: result.content, dirty: false },
        });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [backendClient]
  );

  const activeBackendDocId =
    activeContent?.kind === "backend" ? activeContent.file.docId : null;

  const toggleBreakpoint = useCallback(
    (line: number) => {
      if (activeBackendDocId === null) return;
      const docId = activeBackendDocId;
      setBreakpointsByDoc((prev) => {
        const existing = prev[docId] ?? [];
        const next = existing.includes(line)
          ? existing.filter((l) => l !== line)
          : [...existing, line].sort((a, b) => a - b);
        return { ...prev, [docId]: next };
      });
    },
    [activeBackendDocId]
  );

  // Real launch (task #133) -- always starts a fresh session for the
  // active file's own current breakpoint set, matching `desktop/`'s own
  // F5-style "a finished session is treated as gone" convention.
  const dapLaunch = useCallback(() => {
    if (activeBackendDocId === null || !backendClient) return;
    const docId = activeBackendDocId;
    const breakLines = breakpointsByDoc[docId] ?? [];
    setDapSessionByDoc((prev) => ({
      ...prev,
      [docId]: { sessionId: -1, status: "launching" },
    }));
    backendClient
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
  }, [activeBackendDocId, backendClient, breakpointsByDoc]);

  const dapSendCommand = useCallback(
    (method: string) => {
      if (activeBackendDocId === null || !backendClient) return;
      const session = dapSessionByDoc[activeBackendDocId];
      if (!session || session.sessionId < 0) return;
      backendClient
        .call(method, { session_id: session.sessionId })
        .catch((err: Error) => console.error(`${method} failed:`, err));
    },
    [activeBackendDocId, backendClient, dapSessionByDoc]
  );

  const dapStop = useCallback(() => {
    if (activeBackendDocId === null) return;
    const docId = activeBackendDocId;
    const session = dapSessionByDoc[docId];
    if (session && session.sessionId >= 0 && backendClient) {
      backendClient.call("dap_disconnect", { session_id: session.sessionId }).catch(() => {});
    }
    setDapSessionByDoc((prev) => {
      const next = { ...prev };
      delete next[docId];
      return next;
    });
  }, [activeBackendDocId, backendClient, dapSessionByDoc]);

  const handleContentChange = useCallback((path: string, content: string, saved?: boolean) => {
    setActiveContent((prev) => {
      if (!prev || prev.file.path !== path) return prev;
      return { ...prev, file: { ...prev.file, content, dirty: saved ? false : true } } as ActiveContent;
    });
  }, []);

  if (!isFileSystemAccessSupported()) {
    return (
      <div className="app-root">
        <div className="empty-state">
          The File System Access API isn&apos;t available in this browser. This is a real,
          named platform limit (see web/README.md) -- it currently works in Chromium-based
          browsers (Chrome, Edge, Opera) only; Firefox and Safari don&apos;t implement it.
        </div>
      </div>
    );
  }

  return (
    <div className="app-root">
      <div className="toolbar">
        <span className="toolbar-brand mono">
          <span className="toolbar-brand-glyph" />
          SPARTAN
          <span className="toolbar-brand-suffix">web</span>
        </span>
        <button className="toolbar-btn toolbar-btn-primary sf-chamfer-sm" onClick={openFolder}>
          Open Folder…
        </button>
        <span className="toolbar-note">
          {backendReady
            ? "Connected to a local devserver -- git and backend-mode editing (with live LSP diagnostics and DAP debugging) are live, no Leo yet"
            : "Client-side only in this increment -- no LSP/DAP/Leo/git yet, see README.md"}
        </span>
        <select
          className="toolbar-btn"
          value={theme}
          onChange={(e) => setTheme(e.target.value as ThemeName)}
          title="Theme"
        >
          <option value="SpartanDark">Spartan Dark</option>
          <option value="SpartanLight">Spartan Light</option>
        </select>
        <input
          className="toolbar-btn mono"
          type="text"
          placeholder="Font (default: JetBrains Mono)"
          defaultValue={fontFamily}
          key={fontFamily}
          onBlur={(e) => setFontFamily(e.target.value.trim())}
          style={{ width: 180 }}
        />
      </div>
      <div className="main-body">
        {availableSidebarViews.length > 0 && (
          <div className="file-tree-panel">
            {availableSidebarViews.length > 1 && (
              <div className="sidebar-toggle-row">
                {availableSidebarViews.includes("files") && (
                  <button
                    className={`sidebar-toggle-btn ${activeSidebarView === "files" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("files")}
                  >
                    Files
                  </button>
                )}
                {availableSidebarViews.includes("git") && (
                  <button
                    className={`sidebar-toggle-btn ${activeSidebarView === "git" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("git")}
                  >
                    Git
                  </button>
                )}
                {availableSidebarViews.includes("backend") && (
                  <button
                    className={`sidebar-toggle-btn ${activeSidebarView === "backend" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("backend")}
                  >
                    Backend
                  </button>
                )}
                {availableSidebarViews.includes("models") && (
                  <button
                    className={`sidebar-toggle-btn ${activeSidebarView === "models" ? "sidebar-toggle-active" : ""}`}
                    onClick={() => setSidebarView("models")}
                  >
                    Models
                  </button>
                )}
              </div>
            )}
            {activeSidebarView === "files" && root ? (
              <FileTree root={root} onOpenFile={openFile} />
            ) : activeSidebarView === "git" && backendReady && backendClient?.projectRoot ? (
              <GitPanel client={backendClient} root={backendClient.projectRoot} />
            ) : activeSidebarView === "backend" && backendReady && backendClient?.projectRoot ? (
              <BackendFileTree
                client={backendClient}
                root={backendClient.projectRoot}
                onOpenFile={openBackendFile}
              />
            ) : activeSidebarView === "models" && backendConnected && backendClient ? (
              <ModelsPanel client={backendClient} />
            ) : null}
          </div>
        )}
        <div className="editor-and-debug">
          {activeContent?.kind === "backend" && (
            <DebugPanel
              hasFile
              session={activeBackendDocId !== null ? (dapSessionByDoc[activeBackendDocId] ?? null) : null}
              onLaunch={dapLaunch}
              onContinue={() => dapSendCommand("dap_continue")}
              onStepOver={() => dapSendCommand("dap_step_over")}
              onStepInto={() => dapSendCommand("dap_step_into")}
              onStop={dapStop}
            />
          )}
          <div className="content-area">
            {error && <div className="tree-error">{error}</div>}
            {!error && activeContent?.kind === "local" && (
              <Editor file={activeContent.file} onContentChange={handleContentChange} />
            )}
            {!error && activeContent?.kind === "backend" && backendClient && (
              <BackendEditor
                client={backendClient}
                file={activeContent.file}
                onContentChange={handleContentChange}
                diagnostics={diagnosticsByDoc[activeContent.file.docId]}
                breakpoints={breakpointsByDoc[activeContent.file.docId] ?? []}
                onToggleBreakpoint={toggleBreakpoint}
                stoppedLine={
                  dapSessionByDoc[activeContent.file.docId]?.status === "stopped"
                    ? (dapSessionByDoc[activeContent.file.docId]?.stopped?.frame?.line ?? null)
                    : null
                }
              />
            )}
            {!error && !activeContent && (
              <div className="empty-state">
                {root || backendReady
                  ? "Select a file to open it."
                  : "Open a folder to get started."}
              </div>
            )}
          </div>
        </div>
      </div>
      <div className="status-bar mono">
        {activeContent ? (
          <>
            <span>{activeContent.file.path}</span>
            <span>{activeContent.file.dirty ? "● unsaved" : "saved"}</span>
          </>
        ) : (
          <span>No file open</span>
        )}
        <span
          className="status-backend"
          data-status={backendStatus}
          title={
            backendStatus === "connected"
              ? "Connected to a local spartan-devserver (backend capabilities available)"
              : backendStatus === "client-only"
                ? "No backend detected -- running fully client-side"
                : "Checking for a local spartan-devserver…"
          }
        >
          backend: {backendStatus}
        </span>
        {activeContent?.kind === "backend" &&
          (() => {
            const diags = diagnosticsByDoc[activeContent.file.docId];
            if (diags === undefined) return null;
            const errorCount = diags.filter((d) => d.severity === "error").length;
            const warningCount = diags.filter((d) => d.severity === "warning").length;
            return (
              <span
                className="status-lsp-summary"
                title={`${errorCount} error${errorCount === 1 ? "" : "s"}, ${warningCount} warning${
                  warningCount === 1 ? "" : "s"
                }`}
              >
                {errorCount > 0 && <span className="status-lsp-errors">⛔ {errorCount}</span>}
                {warningCount > 0 && <span className="status-lsp-warnings">⚠ {warningCount}</span>}
                {errorCount === 0 && warningCount === 0 && (
                  <span className="status-lsp-clean">✓ LSP</span>
                )}
              </span>
            );
          })()}
        {androidInfo?.isAndroidProject && (
          <button
            className="status-android-badge"
            type="button"
            disabled={androidBuild?.phase === "building"}
            onClick={buildApk}
            title={`Gradle: ${androidInfo.gradlePath ?? "not found"}${
              androidInfo.gradleVersion ? ` (${androidInfo.gradleVersion})` : ""
            } | SDK: ${androidInfo.sdkRoot ?? "not found"} | adb: ${
              androidInfo.adbPath ?? "not found"
            }${
              androidBuild?.phase === "building" && androidBuild.lastLine
                ? `\n${androidBuild.lastLine}`
                : androidBuild?.phase === "ready"
                  ? `\nBuilt: ${androidBuild.apkPath}`
                  : androidBuild?.phase === "failed"
                    ? `\n${androidBuild.error}`
                    : "\nClick to build a real debug APK (gradle assembleDebug)."
            }`}
          >
            {androidBuild?.phase === "building"
              ? "🤖 Building…"
              : androidBuild?.phase === "ready"
                ? "🤖 ✓ built"
                : androidBuild?.phase === "failed"
                  ? "🤖 ✗ failed"
                  : "🤖 Android"}
          </button>
        )}
        {androidBuild?.phase === "ready" && (
          <button
            className="status-android-badge"
            type="button"
            disabled={androidInstall?.phase === "installing"}
            onClick={installApk}
            title={
              (androidDevices === undefined
                ? "Click to list real attached devices and install the built APK."
                : androidDevices.length === 0
                  ? "No real device attached (adb devices -l reported none)."
                  : `Devices: ${androidDevices
                      .map((d) => `${d.serial} (${d.state}${d.model ? `, ${d.model}` : ""})`)
                      .join(", ")}`) +
              (androidInstall?.phase === "installing" && androidInstall.lastLine
                ? `\n${androidInstall.lastLine}`
                : androidInstall?.phase === "ready"
                  ? "\nInstalled."
                  : androidInstall?.phase === "failed"
                    ? `\n${androidInstall.error}`
                    : "")
            }
          >
            {androidInstall?.phase === "installing"
              ? "📲 Installing…"
              : androidInstall?.phase === "ready"
                ? "📲 ✓ installed"
                : androidInstall?.phase === "failed"
                  ? "📲 ✗ failed"
                  : "📲 Install"}
          </button>
        )}
      </div>
    </div>
  );
}
