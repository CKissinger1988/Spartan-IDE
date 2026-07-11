import React, { useCallback, useState } from "react";
import FileTree from "./components/FileTree";
import Editor, { type OpenFile } from "./components/Editor";
import { ensureBufferWasmInit, Document as WasmDocument } from "./buffer";
import { isFileSystemAccessSupported, pickProjectDirectory, readFileText } from "./fsAccess";

/**
 * Real top-level shell for the web app's first real increment (task #81,
 * §75.89) -- the pure client-side half of the hybrid architecture
 * (§75.85): real local folder/file access via the File System Access
 * API, real editing backed by the real `spartan-buffer` engine compiled
 * to WASM, real save-to-disk, real (single-step) undo.
 *
 * **Real, deliberately out-of-scope in this first increment, named
 * honestly rather than silently missing**: LSP, DAP, Leo, and git are
 * not wired to anything here. `spartan-backend`'s real WebSocket
 * transport (§75.88) exists and is real, tested, production code, but
 * connecting to it needs a real answer to the token-delivery design
 * question that transport's own doc comment explicitly left open (how a
 * browser tab legitimately learns the per-process token and the correct
 * origin) -- not guessed at here. Multi-file tabs are also not built yet
 * -- only one file open at a time, the same real, narrow first-increment
 * scoping this project's own history already applies elsewhere (e.g.
 * `gui-builder`'s own real v1 cuts, §75.38).
 */
export default function App(): React.ReactElement {
  const [root, setRoot] = useState<FileSystemDirectoryHandle | null>(null);
  const [file, setFile] = useState<OpenFile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [wasmReady, setWasmReady] = useState(false);

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
        <span className="toolbar-title mono">SPARTAN (web)</span>
        <button className="toolbar-btn" onClick={openFolder}>
          Open Folder…
        </button>
        <span className="toolbar-note">
          Client-side only in this increment -- no LSP/DAP/Leo/git yet, see README.md
        </span>
      </div>
      <div className="main-body">
        {root && (
          <div className="file-tree-panel">
            <FileTree root={root} onOpenFile={openFile} />
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
      </div>
    </div>
  );
}
