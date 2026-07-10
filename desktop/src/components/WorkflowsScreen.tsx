import React, { useCallback, useState } from "react";
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
          colorMode="dark"
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
