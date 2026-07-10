import React, { useCallback, useEffect, useState } from "react";
import Sidebar from "./components/Sidebar";
import FileTree from "./components/FileTree";
import GitPanel from "./components/GitPanel";
import TabBar from "./components/TabBar";
import StatusBar from "./components/StatusBar";
import Editor, { type OpenFile } from "./components/Editor";
import Placeholder from "./components/Placeholder";
import WorkflowsScreen from "./components/WorkflowsScreen";
import DesignScreen from "./components/DesignScreen";
import ConsoleScreen from "./components/ConsoleScreen";
import SessionsScreen from "./components/SessionsScreen";
import SettingsScreen from "./components/SettingsScreen";
import DevContainersScreen from "./components/DevContainersScreen";
import LeoChatPanel from "./components/LeoChatPanel";
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
                </div>
                {sidebarView === "files" ? (
                  <FileTree root={ROOT} onOpenFile={openFile} />
                ) : (
                  <GitPanel root={ROOT} />
                )}
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
    </div>
  );
}
