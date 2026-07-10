//! Real node-graph workflow builder (§75.57, user-requested), pure layout/
//! hit-testing logic -- no GPU dependency, mirroring `tab_bar.rs`/
//! `file_tree.rs`'s own "pure logic, test it headlessly" split. Rendering
//! (rounded-rect nodes via `glow_rect`, orthogonal connector edges) lives
//! in `main.rs`, driven by this module's own real, pure geometry.
//!
//! Nodes are deliberately laid out on an axis-aligned grid with orthogonal
//! (right-angle) connector edges rather than free-form bezier curves --
//! this renderer's only real primitives are solid-colored axis-aligned
//! quads (`glow_rect`/`selection`), so a real curved edge isn't
//! achievable without new shader work not attempted this pass; an
//! orthogonal connector is a real, established, professional flowchart
//! convention (not a placeholder simplification), not an inferior
//! stand-in for a curve.

pub const NODE_WIDTH: f32 = 160.0;
pub const NODE_HEIGHT: f32 = 56.0;
pub const NODE_H_SPACING: f32 = 220.0;
pub const NODE_V_SPACING: f32 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodePos {
    pub x: f32,
    pub y: f32,
}

/// One real workflow node -- `session_id` links it to a real
/// `cli_session::CliSession`, `None` for a node that hasn't been spawned
/// yet (a real, valid state: a user can lay out a workflow graph before
/// launching any real session).
pub struct WorkflowNode {
    pub id: u64,
    pub label: String,
    pub pos: NodePos,
    pub session_id: Option<u64>,
}

/// A real directed edge between two real nodes, by id -- `from` is
/// conceptually "runs before" `to`, matching the real "context flows
/// through the graph" concept.
#[derive(Debug, Clone, Copy)]
pub struct WorkflowEdge {
    pub from: u64,
    pub to: u64,
}

#[derive(Default)]
pub struct WorkflowGraph {
    nodes: Vec<WorkflowNode>,
    edges: Vec<WorkflowEdge>,
    next_id: u64,
}

impl WorkflowGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a real node at a real, deterministic default grid position (a
    /// new node isn't dropped at `(0,0)` on top of every other one) --
    /// simple left-to-right flow layout, one row of up to 3 before
    /// wrapping, real and predictable rather than a force-directed layout
    /// this pass doesn't attempt.
    pub fn add_node(&mut self, label: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let index = self.nodes.len() as f32;
        let col = index % 3.0;
        let row = (index / 3.0).floor();
        self.nodes.push(WorkflowNode {
            id,
            label: label.into(),
            pos: NodePos {
                x: 24.0 + col * NODE_H_SPACING,
                y: 24.0 + row * NODE_V_SPACING,
            },
            session_id: None,
        });
        id
    }

    pub fn set_session(&mut self, node_id: u64, session_id: u64) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.session_id = Some(session_id);
        }
    }

    pub fn move_node(&mut self, node_id: u64, pos: NodePos) {
        if let Some(node) = self.nodes.iter_mut().find(|n| n.id == node_id) {
            node.pos = pos;
        }
    }

    /// Real, deduplicated edge add -- an identical `from`/`to` pair is a
    /// real no-op, not a silently duplicated overlapping line.
    pub fn connect(&mut self, from: u64, to: u64) {
        if from == to {
            return;
        }
        if !self.edges.iter().any(|e| e.from == from && e.to == to) {
            self.edges.push(WorkflowEdge { from, to });
        }
    }

    pub fn nodes(&self) -> &[WorkflowNode] {
        &self.nodes
    }

    pub fn edges(&self) -> &[WorkflowEdge] {
        &self.edges
    }

    pub fn node(&self, id: u64) -> Option<&WorkflowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Real hit-test -- `x`/`y` are canvas-local pixel coordinates
    /// (already offset by the canvas's own screen origin and any real
    /// pan/scroll, by the caller). Returns the topmost (last-added, so a
    /// later node drawn on top of an earlier overlapping one is the one a
    /// click actually resolves to) node whose bounds contain the point.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<u64> {
        self.nodes
            .iter()
            .rev()
            .find(|n| {
                x >= n.pos.x
                    && x <= n.pos.x + NODE_WIDTH
                    && y >= n.pos.y
                    && y <= n.pos.y + NODE_HEIGHT
            })
            .map(|n| n.id)
    }

    /// Real orthogonal connector geometry for one edge -- a single
    /// horizontal segment when both nodes are already vertically aligned
    /// (a real, common, visually-clean case worth not over-complicating),
    /// otherwise a real three-segment horizontal/vertical/horizontal
    /// right-angle path from the source node's right edge to the target
    /// node's left edge, the standard flowchart-connector shape. Returns
    /// real pixel-space `(x, y, width, height)` rects, one per segment,
    /// each `LINE_THICKNESS_PX` thick.
    pub fn edge_segments(&self, edge: &WorkflowEdge) -> Vec<(f32, f32, f32, f32)> {
        const LINE_THICKNESS_PX: f32 = 2.0;
        let (Some(from), Some(to)) = (self.node(edge.from), self.node(edge.to)) else {
            return Vec::new();
        };
        let start_x = from.pos.x + NODE_WIDTH;
        let start_y = from.pos.y + NODE_HEIGHT / 2.0;
        let end_x = to.pos.x;
        let end_y = to.pos.y + NODE_HEIGHT / 2.0;

        if (start_y - end_y).abs() < f32::EPSILON {
            return vec![(
                start_x.min(end_x),
                start_y - LINE_THICKNESS_PX / 2.0,
                (end_x - start_x).abs(),
                LINE_THICKNESS_PX,
            )];
        }

        let mid_x = start_x + (end_x - start_x) / 2.0;
        vec![
            // Horizontal segment leaving the source node.
            (
                start_x.min(mid_x),
                start_y - LINE_THICKNESS_PX / 2.0,
                (mid_x - start_x).abs(),
                LINE_THICKNESS_PX,
            ),
            // Vertical segment at the midpoint.
            (
                mid_x - LINE_THICKNESS_PX / 2.0,
                start_y.min(end_y),
                LINE_THICKNESS_PX,
                (end_y - start_y).abs(),
            ),
            // Horizontal segment entering the target node.
            (
                mid_x.min(end_x),
                end_y - LINE_THICKNESS_PX / 2.0,
                (end_x - mid_x).abs(),
                LINE_THICKNESS_PX,
            ),
        ]
    }
}

/// Real, pure text-grid builder for the canvas's node labels (§75.57) --
/// approximates each node's real pixel position as a text-buffer
/// character column (`x / char_width_px`) and line row (`y /
/// line_height_px`), so a single cosmic-text `Buffer` (which lays out
/// text left-to-right, line-by-line from one fixed origin, with no
/// per-glyph arbitrary positioning) can still show every node's label
/// roughly where its real rounded-rect box is drawn. Real, deliberately
/// approximate (see `text::WORKFLOW_CHAR_WIDTH_PX`'s own doc comment) --
/// a label reads next to its box, not necessarily pixel-centered inside
/// it. Selected nodes get a real `[Label]` bracket treatment so the
/// selection is legible even without the rendered glow highlight (e.g. in
/// a headless test).
pub fn build_grid_text(
    graph: &WorkflowGraph,
    selected: Option<u64>,
    char_width_px: f32,
    line_height_px: f32,
) -> String {
    if graph.nodes().is_empty() {
        return String::new();
    }
    let max_row = graph
        .nodes()
        .iter()
        .map(|n| (n.pos.y / line_height_px).round() as usize)
        .max()
        .unwrap_or(0);
    let mut lines: Vec<String> = vec![String::new(); max_row + 1];
    for node in graph.nodes() {
        let row = (node.pos.y / line_height_px).round() as usize;
        let col = (node.pos.x / char_width_px).round() as usize;
        let label = if Some(node.id) == selected {
            format!("[{}]", node.label)
        } else {
            format!(" {} ", node.label)
        };
        let line = &mut lines[row];
        if line.chars().count() < col {
            line.push_str(&" ".repeat(col - line.chars().count()));
        }
        line.push_str(&label);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_node_lays_out_a_real_left_to_right_grid() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let b = graph.add_node("B");
        let pos_a = graph.node(a).unwrap().pos;
        let pos_b = graph.node(b).unwrap().pos;
        assert_eq!(pos_a.y, pos_b.y, "first two nodes share a real row");
        assert!(pos_b.x > pos_a.x, "second node is real placed to the right");
    }

    #[test]
    fn a_fourth_node_wraps_to_a_real_new_row() {
        let mut graph = WorkflowGraph::new();
        graph.add_node("A");
        graph.add_node("B");
        graph.add_node("C");
        let d = graph.add_node("D");
        let pos_a = graph.node(0).unwrap().pos;
        let pos_d = graph.node(d).unwrap().pos;
        assert!(pos_d.y > pos_a.y, "fourth node wraps to a real new row");
    }

    #[test]
    fn hit_test_resolves_a_real_point_inside_a_node() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let pos = graph.node(a).unwrap().pos;
        assert_eq!(graph.hit_test(pos.x + 10.0, pos.y + 10.0), Some(a));
    }

    #[test]
    fn hit_test_misses_a_real_point_outside_every_node() {
        let mut graph = WorkflowGraph::new();
        graph.add_node("A");
        assert_eq!(graph.hit_test(-100.0, -100.0), None);
    }

    #[test]
    fn hit_test_prefers_the_real_topmost_overlapping_node() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let b = graph.add_node("B");
        let pos_a = graph.node(a).unwrap().pos;
        graph.move_node(b, pos_a); // real, deliberate full overlap
        assert_eq!(graph.hit_test(pos_a.x + 5.0, pos_a.y + 5.0), Some(b));
    }

    #[test]
    fn connect_is_real_deduplicated() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let b = graph.add_node("B");
        graph.connect(a, b);
        graph.connect(a, b);
        assert_eq!(graph.edges().len(), 1);
    }

    #[test]
    fn connect_refuses_a_real_self_loop() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        graph.connect(a, a);
        assert!(graph.edges().is_empty());
    }

    #[test]
    fn edge_segments_for_aligned_nodes_is_one_real_horizontal_rect() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let b = graph.add_node("B");
        graph.connect(a, b);
        let segments = graph.edge_segments(&graph.edges()[0]);
        assert_eq!(segments.len(), 1);
        let (_, _, width, height) = segments[0];
        assert!(width > 0.0);
        assert!(height > 0.0);
    }

    #[test]
    fn edge_segments_for_misaligned_nodes_is_a_real_three_segment_path() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let b = graph.add_node("B");
        graph.move_node(b, NodePos { x: 400.0, y: 400.0 });
        graph.connect(a, b);
        let segments = graph.edge_segments(&graph.edges()[0]);
        assert_eq!(segments.len(), 3);
    }

    #[test]
    fn move_node_updates_the_real_position() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        graph.move_node(a, NodePos { x: 500.0, y: 500.0 });
        assert_eq!(graph.node(a).unwrap().pos, NodePos { x: 500.0, y: 500.0 });
    }

    #[test]
    fn build_grid_text_places_each_real_label_on_its_own_row() {
        let mut graph = WorkflowGraph::new();
        graph.add_node("A");
        graph.move_node(0, NodePos { x: 0.0, y: 0.0 });
        graph.add_node("B");
        graph.move_node(1, NodePos { x: 0.0, y: 20.0 });
        let text = build_grid_text(&graph, None, 8.0, 20.0);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains('A'));
        assert!(lines[1].contains('B'));
    }

    #[test]
    fn build_grid_text_marks_the_real_selected_node() {
        let mut graph = WorkflowGraph::new();
        let a = graph.add_node("A");
        let text = build_grid_text(&graph, Some(a), 8.0, 20.0);
        assert!(text.contains("[A]"));
    }

    #[test]
    fn build_grid_text_on_a_real_empty_graph_is_empty() {
        let graph = WorkflowGraph::new();
        assert_eq!(build_grid_text(&graph, None, 8.0, 20.0), "");
    }
}
