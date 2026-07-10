import React, { useCallback, useState } from "react";
import FileTree from "./components/FileTree";
import TabBar from "./components/TabBar";
import ModeToggle, { type Mode } from "./components/ModeToggle";
import StatusBar from "./components/StatusBar";
import Editor, { type OpenFile } from "./components/Editor";
import "./app.css";

const ROOT = new URLSearchParams(window.location.search).get("root") ?? "/";

export default function App(): React.ReactElement {
  const [files, setFiles] = useState<OpenFile[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [mode, setMode] = useState<Mode>("Editor");

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
    },
    [files]
  );

  const activeFile = files[activeIndex] ?? null;

  return (
    <div className="app-root">
      <div className="activity-bar mono">
        <span className="activity-icon activity-active">Files</span>
        <span className="activity-icon">Git</span>
        <span className="activity-icon">Agent</span>
        <span className="activity-icon">Set</span>
      </div>
      <div className="sidebar">
        <FileTree root={ROOT} onOpenFile={openFile} />
      </div>
      <div className="main-column">
        <div className="top-row">
          <TabBar files={files} activeIndex={activeIndex} onSelect={setActiveIndex} onClose={closeFile} />
          <ModeToggle mode={mode} onChange={setMode} />
        </div>
        <div className="content-area">
          {mode === "Editor" && activeFile && (
            <Editor file={activeFile} onContentChange={handleContentChange} />
          )}
          {mode === "Editor" && !activeFile && (
            <div className="empty-state mono">Open a file from the sidebar to start editing.</div>
          )}
          {mode !== "Editor" && (
            <div className="empty-state mono">
              {mode} mode is real in the original wgpu shell (crates/spartan-editor-core) but not
              yet ported to this new Electron shell -- named honestly, not simulated.
            </div>
          )}
        </div>
        <StatusBar fileCount={files.length} activePath={activeFile?.path ?? null} />
      </div>
    </div>
  );
}
