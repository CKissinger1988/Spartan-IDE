//! Real file tree sidebar logic (§75.26, task #24) -- real `std::fs`
//! directory traversal and real expand/collapse state, producing an
//! ordered list of visible rows a renderer can turn into one line of text
//! per row and one real cosmic-text hit-test per click, the same "pure
//! layout logic, no GPU dependency" split `tab_bar.rs` already established.
//! Re-reads the filesystem from scratch on every `visible_rows()` call --
//! a real, deliberate v1 simplification (no caching, no filesystem
//! watching), named rather than hidden in §75.26.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One row in the currently-visible (accounting for expand state) tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    /// Only meaningful when `is_dir` -- whether this directory's own
    /// children are currently included in the row list.
    pub is_expanded: bool,
}

/// Real, live directory-tree state: a root directory plus which of its
/// subdirectories are currently expanded. Files are always real (never a
/// synthetic fixture) -- the tree has nothing to show for a
/// `--synthetic:<n>` in-memory-only document.
pub struct FileTree {
    root: PathBuf,
    expanded: BTreeSet<PathBuf>,
}

impl FileTree {
    /// The root starts collapsed (no directory expanded) -- deliberately
    /// not auto-expanding anything, including itself recursively, since a
    /// real project can be arbitrarily deep and eagerly expanding it would
    /// both be slow and produce an overwhelming initial view. `visible_rows`
    /// still shows the root's own immediate children even with an empty
    /// `expanded` set, since those aren't gated by any expand state -- only
    /// a subdirectory's own children are.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            expanded: BTreeSet::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Toggles a directory's expand state. A no-op (silently) if `path`
    /// isn't actually a directory under this tree's root -- callers resolve
    /// `path` from a real `visible_rows()` row, so this should never
    /// actually happen in practice, but toggling an arbitrary path is
    /// harmless either way (it just never appears in any future
    /// `visible_rows()` call, since nothing under it is ever listed).
    pub fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
    }

    /// Real, recursive (through expanded directories only) directory
    /// listing, freshly read from disk every call. Directories sort before
    /// files at each level; both sort case-sensitively by name within their
    /// own group -- simple, deterministic, real `std::fs` metadata, not a
    /// guess. A directory this process can't read (permissions, since
    /// removed, ...) is silently skipped rather than panicking or aborting
    /// the whole listing -- matching this crate's established "a real I/O
    /// failure degrades gracefully" pattern (e.g. `close_file`, LSP/DAP
    /// spawn failures) rather than being a new exception to it.
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        Self::collect(&self.root, 0, &self.expanded, &mut rows);
        rows
    }

    fn collect(dir: &Path, depth: usize, expanded: &BTreeSet<PathBuf>, rows: &mut Vec<TreeRow>) {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
        entries.sort_by(|a, b| {
            let a_is_dir = a.path().is_dir();
            let b_is_dir = b.path().is_dir();
            b_is_dir
                .cmp(&a_is_dir)
                .then_with(|| a.file_name().cmp(&b.file_name()))
        });
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = path.is_dir();
            let is_expanded = is_dir && expanded.contains(&path);
            rows.push(TreeRow {
                path: path.clone(),
                name,
                is_dir,
                depth,
                is_expanded,
            });
            if is_expanded {
                Self::collect(&path, depth + 1, expanded, rows);
            }
        }
    }
}

/// Builds the sidebar's real display text from a real row list -- one line
/// per row, indented two spaces per depth level, prefixed with `"> "`
/// (collapsed directory), `"v "` (expanded directory), or `"  "` (a file,
/// no marker) -- plain ASCII rather than Unicode disclosure triangles, so
/// this never depends on the active font actually covering them.
pub fn build_tree_text(rows: &[TreeRow]) -> String {
    rows.iter()
        .map(|row| {
            let indent = "  ".repeat(row.depth);
            let marker = if row.is_dir {
                if row.is_expanded {
                    "v "
                } else {
                    "> "
                }
            } else {
                "  "
            };
            format!("{indent}{marker}{}", row.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Real temp-directory fixture (not a mock filesystem) -- matches this
    /// crate's own established preference for exercising real I/O rather
    /// than an abstraction over it, the same reasoning `dap_integration.rs`/
    /// `lsp_integration.rs` spawn real subprocesses instead of stubbing
    /// them. Cleaned up on drop via `tempfile`... no such dependency exists
    /// here, so this builds and tears down a real directory under
    /// `std::env::temp_dir()` by hand instead of adding one just for tests.
    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new(unique: &str) -> Self {
            let root = std::env::temp_dir().join(format!("spartan_file_tree_test_{unique}"));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn visible_rows_lists_the_root_s_immediate_children_by_default() {
        let tmp = TempTree::new("basic");
        fs::write(tmp.root.join("b.rs"), "").unwrap();
        fs::write(tmp.root.join("a.rs"), "").unwrap();
        fs::create_dir(tmp.root.join("sub")).unwrap();

        let tree = FileTree::new(tmp.root.clone());
        let rows = tree.visible_rows();

        // Directories sort before files; both alphabetically within their
        // own group -- "sub" (dir) first, then "a.rs", "b.rs".
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "sub");
        assert!(rows[0].is_dir);
        assert!(!rows[0].is_expanded);
        assert_eq!(rows[1].name, "a.rs");
        assert!(!rows[1].is_dir);
        assert_eq!(rows[2].name, "b.rs");
    }

    #[test]
    fn collapsed_subdirectories_do_not_show_their_children() {
        let tmp = TempTree::new("collapsed");
        fs::create_dir(tmp.root.join("sub")).unwrap();
        fs::write(tmp.root.join("sub").join("inner.rs"), "").unwrap();

        let tree = FileTree::new(tmp.root.clone());
        let rows = tree.visible_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "sub");
    }

    #[test]
    fn toggling_a_directory_reveals_its_children_at_depth_plus_one() {
        let tmp = TempTree::new("expand");
        fs::create_dir(tmp.root.join("sub")).unwrap();
        fs::write(tmp.root.join("sub").join("inner.rs"), "").unwrap();

        let mut tree = FileTree::new(tmp.root.clone());
        let sub_path = tmp.root.join("sub");
        tree.toggle(&sub_path);
        let rows = tree.visible_rows();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "sub");
        assert!(rows[0].is_expanded);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "inner.rs");
        assert_eq!(rows[1].depth, 1);
    }

    #[test]
    fn toggling_twice_collapses_it_again() {
        let tmp = TempTree::new("retoggle");
        fs::create_dir(tmp.root.join("sub")).unwrap();
        fs::write(tmp.root.join("sub").join("inner.rs"), "").unwrap();

        let mut tree = FileTree::new(tmp.root.clone());
        let sub_path = tmp.root.join("sub");
        tree.toggle(&sub_path);
        tree.toggle(&sub_path);
        let rows = tree.visible_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "sub");
        assert!(!rows[0].is_expanded);
    }

    #[test]
    fn nested_expansion_shows_grandchildren_at_depth_two() {
        let tmp = TempTree::new("nested");
        fs::create_dir(tmp.root.join("a")).unwrap();
        fs::create_dir(tmp.root.join("a").join("b")).unwrap();
        fs::write(tmp.root.join("a").join("b").join("c.rs"), "").unwrap();

        let mut tree = FileTree::new(tmp.root.clone());
        tree.toggle(&tmp.root.join("a"));
        tree.toggle(&tmp.root.join("a").join("b"));
        let rows = tree.visible_rows();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].name, "a");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].name, "b");
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].name, "c.rs");
        assert_eq!(rows[2].depth, 2);
    }

    #[test]
    fn an_unreadable_root_returns_an_empty_list_not_a_panic() {
        let tree = FileTree::new(PathBuf::from("/this/path/does/not/exist/anywhere"));
        assert_eq!(tree.visible_rows(), Vec::new());
    }

    #[test]
    fn build_tree_text_uses_the_expected_markers_and_indentation() {
        let rows = vec![
            TreeRow {
                path: PathBuf::from("/root/sub"),
                name: "sub".to_string(),
                is_dir: true,
                depth: 0,
                is_expanded: true,
            },
            TreeRow {
                path: PathBuf::from("/root/sub/inner.rs"),
                name: "inner.rs".to_string(),
                is_dir: false,
                depth: 1,
                is_expanded: false,
            },
            TreeRow {
                path: PathBuf::from("/root/other"),
                name: "other".to_string(),
                is_dir: true,
                depth: 0,
                is_expanded: false,
            },
        ];
        assert_eq!(build_tree_text(&rows), "v sub\n    inner.rs\n> other");
    }

    #[test]
    fn build_tree_text_on_an_empty_row_list_is_empty() {
        assert_eq!(build_tree_text(&[]), "");
    }
}
