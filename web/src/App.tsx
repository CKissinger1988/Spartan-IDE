import React, { useCallback, useEffect, useState } from "react";
import FileTree from "./components/FileTree";
import GitPanel from "./components/GitPanel";
import Editor, { type OpenFile } from "./components/Editor";
import { ensureBufferWasmInit, Document as WasmDocument } from "./buffer";
import { isFileSystemAccessSupported, pickProjectDirectory, readFileText } from "./fsAccess";
import { applyTheme, type ThemeName } from "./applyTheme";
import { applyFontFamily } from "./applyFontFamily";
import { BackendClient } from "./backendClient";

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
 * honestly rather than silently missing**: LSP, DAP, and Leo are still
 * not wired to anything here. `spartan-backend`'s real WebSocket
 * transport (§75.88) exists and is real, tested, production code; a later
 * increment answered the token-delivery design question that transport's
 * own doc comment explicitly left open (how a browser tab legitimately
 * learns the per-process token and the correct origin -- the
 * `/__spartan/session` same-origin handoff), and a further increment used
 * that same handoff to advertise the devserver's own real project root, so
 * **git is now real and wired** when a devserver is connected: see
 * `GitPanel`, toggled alongside the File System Access-backed `FileTree`.
 * Multi-file tabs are also not built yet -- only one file open at a time,
 * the same real, narrow first-increment scoping this project's own
 * history already applies elsewhere (e.g. `gui-builder`'s own real v1
 * cuts, §75.38).
 */
export default function App(): React.ReactElement {
  const [root, setRoot] = useState<FileSystemDirectoryHandle | null>(null);
  const [file, setFile] = useState<OpenFile | null>(null);
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
  const [sidebarView, setSidebarView] = useState<"files" | "git">("files");

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

  // A real, connected devserver with a real, resolved project root is what
  // makes the Git panel usable -- the File System Access API never gives
  // this app an OS path for whatever folder `root` (above) points at, so
  // git operations run against the devserver's *own* launch directory
  // instead (see `GitPanel`'s and `backendClient.ts`'s own doc comments).
  const canGit = backendStatus === "connected" && !!backendClient?.projectRoot;
  const activeSidebarView: "files" | "git" =
    sidebarView === "git" && canGit ? "git" : root ? "files" : canGit ? "git" : "files";

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
        setFile({ path, handle, doc, content, dirty: false });
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [wasmReady]
  );

  const handleContentChange = useCallback((path: string, content: string, saved?: boolean) => {
    setFile((prev) => {
      if (!prev || prev.path !== path) return prev;
      return { ...prev, content, dirty: saved ? false : true };
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
          {canGit
            ? "Connected to a local devserver -- git is live, no LSP/DAP/Leo yet"
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
        {(root || canGit) && (
          <div className="file-tree-panel">
            {root && canGit && (
              <div className="sidebar-toggle-row">
                <button
                  className={`sidebar-toggle-btn ${activeSidebarView === "files" ? "sidebar-toggle-active" : ""}`}
                  onClick={() => setSidebarView("files")}
                >
                  Files
                </button>
                <button
                  className={`sidebar-toggle-btn ${activeSidebarView === "git" ? "sidebar-toggle-active" : ""}`}
                  onClick={() => setSidebarView("git")}
                >
                  Git
                </button>
              </div>
            )}
            {activeSidebarView === "files" && root ? (
              <FileTree root={root} onOpenFile={openFile} />
            ) : activeSidebarView === "git" && canGit && backendClient?.projectRoot ? (
              <GitPanel client={backendClient} root={backendClient.projectRoot} />
            ) : null}
          </div>
        )}
        <div className="content-area">
          {error && <div className="tree-error">{error}</div>}
          {!error && file && <Editor file={file} onContentChange={handleContentChange} />}
          {!error && !file && (
            <div className="empty-state">
              {root ? "Select a file to open it." : "Open a folder to get started."}
            </div>
          )}
        </div>
      </div>
      <div className="status-bar mono">
        {file ? (
          <>
            <span>{file.path}</span>
            <span>{file.dirty ? "● unsaved" : "saved"}</span>
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
      </div>
    </div>
  );
}
