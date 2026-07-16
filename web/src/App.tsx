import React, { useCallback, useEffect, useState } from "react";
import FileTree from "./components/FileTree";
import GitPanel from "./components/GitPanel";
import BackendFileTree from "./components/BackendFileTree";
import Editor, { type OpenFile } from "./components/Editor";
import BackendEditor, { type BackendOpenFile, type LspDiagnostic } from "./components/BackendEditor";
import { ensureBufferWasmInit, Document as WasmDocument } from "./buffer";
import { isFileSystemAccessSupported, pickProjectDirectory, readFileText } from "./fsAccess";
import { applyTheme, type ThemeName } from "./applyTheme";
import { applyFontFamily } from "./applyFontFamily";
import { BackendClient } from "./backendClient";

type ActiveContent =
  | { kind: "local"; file: OpenFile }
  | { kind: "backend"; file: BackendOpenFile }
  | null;

type SidebarView = "files" | "git" | "backend";

type BackendStatus = "connecting" | "connected" | "client-only";

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
 * honestly rather than silently missing**: DAP and Leo are still not
 * wired to anything here. `spartan-backend`'s real WebSocket transport
 * (§75.88) exists and is real, tested, production code; a later increment
 * answered the token-delivery design question that transport's own doc
 * comment explicitly left open (how a browser tab legitimately learns the
 * per-process token and the correct origin -- the `/__spartan/session`
 * same-origin handoff), a further increment used that same handoff to
 * advertise the devserver's own real project root so **git is real and
 * wired** (`GitPanel`), and a further increment gave this app a real
 * **backend-mode editing path** (`BackendFileTree`/`BackendEditor`,
 * routing file open/edit/undo/redo/save through `BackendClient` instead
 * of File System Access + WASM) purely so `spartan-backend`'s own real
 * LSP diagnostics wiring -- which needs a real `doc_id` the WASM path has
 * no equivalent for -- has something to attach to here, the same way it
 * already does in `desktop/`. The two editing paths are independent and
 * both real: File System Access + WASM (`FileTree`/`Editor`) works with
 * no backend at all; `BackendFileTree`/`BackendEditor` only appear once a
 * devserver is connected with a known project root, and operate on that
 * root, not necessarily the File System Access folder. Multi-file tabs
 * are also not built yet -- only one file open at a time (whichever kind
 * was opened most recently), the same real, narrow first-increment
 * scoping this project's own history already applies elsewhere (e.g.
 * `gui-builder`'s own real v1 cuts, §75.38).
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
  const availableSidebarViews: SidebarView[] = [
    ...(root ? (["files"] as const) : []),
    ...(backendReady ? (["git", "backend"] as const) : []),
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
            ? "Connected to a local devserver -- git and backend-mode editing (with live LSP diagnostics) are live, no DAP/Leo yet"
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
            ) : null}
          </div>
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
      </div>
    </div>
  );
}
