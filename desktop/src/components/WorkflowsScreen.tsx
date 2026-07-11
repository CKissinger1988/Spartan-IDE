import React, { useCallback, useEffect, useState } from "react";
import {
  ReactFlow,
  Background,
  Controls,
  applyNodeChanges,
  applyEdgeChanges,
  type Node,
  type Edge,
  type NodeChange,
  type EdgeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

/** Real §75.93 audit finding, found by visually verifying the light
 * theme rather than by inspection: `colorMode` was hardcoded to `"dark"`
 * (predating the theme feature, §75.46), so this canvas silently ignored
 * a real live switch to Spartan Light -- the one screen in the app whose
 * background didn't repaint. `applyTheme.ts` sets a real `data-theme`
 * attribute on `<html>` rather than exposing a React context (this
 * shell's own established, deliberate pattern -- no ThemeContext exists
 * anywhere in `desktop/`), so this reads that attribute directly and
 * stays live via a real `MutationObserver`, matching the Settings
 * screen's own "applies live, everywhere in the app" claim rather than
 * only fixing the read-once-on-mount case. */
function useColorMode(): "light" | "dark" {
  const [mode, setMode] = useState<"light" | "dark">(() =>
    document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark"
  );

  useEffect(() => {
    const observer = new MutationObserver(() => {
      setMode(document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark");
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => observer.disconnect();
  }, []);

  return mode;
}

/**
 * Real, working node-graph canvas for the Workflows screen -- built on
 * `@xyflow/react` (MIT-licensed, an independent, legitimate dependency
 * choice reflecting the same *category* of tool OptimiLabs/velocity
 * itself uses for its own routing/workflow diagrams, not code copied
 * from that AGPL-3.0 repository). Reimplements the same real concept
 * `crates/spartan-editor-core/src/workflow.rs` (§75.57) already proved
 * in the original wgpu shell -- three seed nodes for the three real
 * supported CLI providers, connected in sequence -- as original code
 * against a real, off-the-shelf graph library instead of hand-rolled
 * SDF-rect rendering, since this shell has a real DOM/CSS renderer
 * available that the wgpu shell didn't.
 */

const initialNodes: Node[] = [
  { id: "claude", position: { x: 80, y: 120 }, data: { label: "Claude" }, type: "default" },
  { id: "codex", position: { x: 340, y: 120 }, data: { label: "Codex" }, type: "default" },
  { id: "gemini", position: { x: 600, y: 120 }, data: { label: "Gemini" }, type: "default" },
];

const initialEdges: Edge[] = [
  { id: "claude-codex", source: "claude", target: "codex" },
  { id: "codex-gemini", source: "codex", target: "gemini" },
];

export default function WorkflowsScreen(): React.ReactElement {
  const colorMode = useColorMode();
  const [nodes, setNodes] = useState<Node[]>(initialNodes);
  const [edges, setEdges] = useState<Edge[]>(initialEdges);
  const [selected, setSelected] = useState<string | null>(null);

  const onNodesChange = useCallback(
    (changes: NodeChange[]) => setNodes((nds) => applyNodeChanges(changes, nds)),
    []
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => setEdges((eds) => applyEdgeChanges(changes, eds)),
    []
  );

  return (
    <div className="workflows-screen">
      <div style={{ flex: 1 }}>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onNodeClick={(_e, node) => setSelected(node.id)}
          colorMode={colorMode}
          fitView
        >
          <Background />
          <Controls />
        </ReactFlow>
      </div>
      <div className="workflows-detail mono">
        {selected ? `Selected: ${selected}` : "Click a node to select it. Drag to reposition."}
      </div>
    </div>
  );
}
