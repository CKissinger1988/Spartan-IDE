//! Real accessibility tree construction (§16.3, task #9) -- pure logic,
//! no GPU/AccessKit-adapter dependency, mirroring `tab_bar.rs`/
//! `file_tree.rs`/`git_panel.rs`'s own "pure layout, test it headlessly"
//! split. This is the actual explicit accessibility tree §16.3 calls for
//! ("since the UI is custom-rendered... this requires an explicit
//! accessibility tree built alongside the wgpu render tree"), not a
//! best-effort afterthought: real node IDs, real roles, and the real,
//! full (not merely windowed/visible-slice) document text and tab list,
//! rebuilt from live state every frame the same way this crate's other
//! per-frame text (tab bar, sidebar, modal) already is.

use accesskit::{Node, NodeBuilder, NodeClassSet, NodeId, Role, Tree, TreeUpdate};

pub const WINDOW_ID: NodeId = NodeId(0);
pub const DOCUMENT_ID: NodeId = NodeId(1);
/// Tab IDs start at 2 and count up by real tab index -- stable across
/// frames as long as the same file stays at the same tab position, which
/// matches AccessKit's own "only nodes that changed need re-sending"
/// efficiency note closely enough for this crate's real tab counts
/// (single digits to low tens, never the kind of scale where a more
/// elaborate stable-ID scheme would matter).
fn tab_node_id(index: usize) -> NodeId {
    NodeId(2 + index as u64)
}

/// One real, minimal description of an open file, enough to build both
/// its tab node and (for the active one) the document node -- kept
/// separate from `OpenFile` itself so this module has no dependency on
/// `editor_view`/GPU state and stays headlessly testable.
pub struct TabInfo<'a> {
    pub label: &'a str,
    pub dirty: bool,
}

/// Real, full document text for the active file goes into the document
/// node's `value` -- deliberately not windowed the way rendering/tree-
/// sitter highlighting are (§75.11), since a screen reader user needs the
/// real content, not just what's currently scrolled into view.
pub fn build_tree(
    window_title: &str,
    tabs: &[TabInfo],
    active: usize,
    active_document_text: &str,
) -> TreeUpdate {
    let mut classes = NodeClassSet::new();

    let mut tab_ids = Vec::with_capacity(tabs.len());
    let mut nodes: Vec<(NodeId, Node)> = Vec::with_capacity(tabs.len() + 2);
    for (index, tab) in tabs.iter().enumerate() {
        let id = tab_node_id(index);
        tab_ids.push(id);
        let mut builder = NodeBuilder::new(Role::Tab);
        let name = if tab.dirty {
            format!("{} (unsaved)", tab.label)
        } else {
            tab.label.to_string()
        };
        builder.set_name(name);
        if index == active {
            builder.set_selected(true);
        }
        nodes.push((id, builder.build(&mut classes)));
    }

    let mut tab_list_builder = NodeBuilder::new(Role::TabList);
    tab_list_builder.set_children(tab_ids);
    let tab_list_id = NodeId(u64::MAX - 1);
    nodes.push((tab_list_id, tab_list_builder.build(&mut classes)));

    let mut document_builder = NodeBuilder::new(Role::Document);
    if let Some(active_tab) = tabs.get(active) {
        document_builder.set_name(active_tab.label);
    }
    document_builder.set_value(active_document_text);
    document_builder.set_read_only();
    nodes.push((DOCUMENT_ID, document_builder.build(&mut classes)));

    let mut root_builder = NodeBuilder::new(Role::Window);
    root_builder.set_name(window_title);
    root_builder.set_children(vec![tab_list_id, DOCUMENT_ID]);
    nodes.push((WINDOW_ID, root_builder.build(&mut classes)));

    let mut tree = Tree::new(WINDOW_ID);
    tree.app_name = Some("Spartan IDE".to_string());
    tree.toolkit_name = Some("spartan-editor-core".to_string());

    TreeUpdate {
        nodes,
        tree: Some(tree),
        focus: DOCUMENT_ID,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tree_produces_a_root_a_tab_list_and_a_document_node() {
        let tabs = vec![
            TabInfo {
                label: "main.rs",
                dirty: false,
            },
            TabInfo {
                label: "lib.rs",
                dirty: true,
            },
        ];
        let update = build_tree("Spartan IDE", &tabs, 0, "fn main() {}");

        let root = update
            .nodes
            .iter()
            .find(|(id, _)| *id == WINDOW_ID)
            .expect("root node must exist")
            .1
            .clone();
        assert_eq!(root.role(), Role::Window);
        assert_eq!(root.name(), Some("Spartan IDE"));

        let document = update
            .nodes
            .iter()
            .find(|(id, _)| *id == DOCUMENT_ID)
            .expect("document node must exist")
            .1
            .clone();
        assert_eq!(document.role(), Role::Document);
        assert_eq!(document.value(), Some("fn main() {}"));
        assert_eq!(document.name(), Some("main.rs"));
        assert!(document.is_read_only());
    }

    #[test]
    fn a_dirty_tab_s_accessible_name_notes_it_is_unsaved() {
        let tabs = vec![TabInfo {
            label: "main.rs",
            dirty: true,
        }];
        let update = build_tree("Spartan IDE", &tabs, 0, "");
        let tab_node = update
            .nodes
            .iter()
            .find(|(id, _)| *id == tab_node_id(0))
            .expect("tab node must exist")
            .1
            .clone();
        assert_eq!(tab_node.name(), Some("main.rs (unsaved)"));
    }

    #[test]
    fn the_active_tab_is_marked_selected_and_others_are_not() {
        let tabs = vec![
            TabInfo {
                label: "a.rs",
                dirty: false,
            },
            TabInfo {
                label: "b.rs",
                dirty: false,
            },
        ];
        let update = build_tree("Spartan IDE", &tabs, 1, "");
        let a = update
            .nodes
            .iter()
            .find(|(id, _)| *id == tab_node_id(0))
            .unwrap()
            .1
            .clone();
        let b = update
            .nodes
            .iter()
            .find(|(id, _)| *id == tab_node_id(1))
            .unwrap()
            .1
            .clone();
        assert_ne!(a.is_selected(), Some(true));
        assert_eq!(b.is_selected(), Some(true));
    }

    #[test]
    fn focus_always_points_at_the_document_node() {
        let update = build_tree("Spartan IDE", &[], 0, "");
        assert_eq!(update.focus, DOCUMENT_ID);
    }

    #[test]
    fn document_text_is_the_real_full_text_not_truncated() {
        let long_text = "line\n".repeat(10_000);
        let tabs = vec![TabInfo {
            label: "big.rs",
            dirty: false,
        }];
        let update = build_tree("Spartan IDE", &tabs, 0, &long_text);
        let document = update
            .nodes
            .iter()
            .find(|(id, _)| *id == DOCUMENT_ID)
            .unwrap()
            .1
            .clone();
        assert_eq!(document.value(), Some(long_text.as_str()));
    }
}
