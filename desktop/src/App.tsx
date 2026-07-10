import React, { useCallback, useState } from "react";
import Sidebar from "./components/Sidebar";
import FileTree from "./components/FileTree";
import TabBar from "./components/TabBar";
import StatusBar from "./components/StatusBar";
import Editor, { type OpenFile } from "./components/Editor";
import Placeholder from "./components/Placeholder";
import WorkflowsScreen from "./components/WorkflowsScreen";
import { NAV, type ScreenId } from "./nav";
import "./app.css";

const ROOT = new URLSearchParams(window.location.search).get("root") ?? "/";

export default function App(): React.ReactElement {
  const [files, setFiles] = useState<OpenFile[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [screen, setScreen] = useState<ScreenId>("editor");

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
  const screenLabel = NAV.flatMap((g) => g.items).find((i) => i.id === screen)?.label ?? screen;

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
                <FileTree root={ROOT} onOpenFile={openFile} />
              </div>
              <div className="content-area">
                {activeFile ? (
                  <Editor file={activeFile} onContentChange={handleContentChange} />
                ) : (
                  <div className="empty-state mono">Open a file from the sidebar to start editing.</div>
                )}
              </div>
            </div>
            <StatusBar fileCount={files.length} activePath={activeFile?.path ?? null} />
          </>
        ) : (
          <>
            <div className="top-row">
              <div className="screen-title mono">{screenLabel}</div>
            </div>
            <div className="content-area">
              {screen === "workflows" ? <WorkflowsScreen /> : <Placeholder screen={screen} />}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
