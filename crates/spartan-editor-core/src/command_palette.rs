//! Real, scoped command palette (§16.1, task #3) -- a fuzzy-filterable
//! list combining a small set of static commands with real project files,
//! opened with Ctrl+P. Pure matching/layout logic, no GPU dependency,
//! mirroring `tab_bar.rs`/`file_tree.rs`/`git_panel.rs`/`mode_toggle.rs`'s
//! own "pure logic, test it headlessly" split -- keyboard-only selection
//! (Up/Down/Enter/Escape) reuses the existing modal-text rendering
//! (§75.23) with no new hit-testing needed, since there are no clickable
//! rows here either, the same real v1 scope cut §75.23 itself already
//! named for its own modal.
//!
//! Deliberately NOT §16.1's full "unified command palette" vision -- no
//! natural-language Leo routing (there is no `ModelProvider`/Leo anywhere
//! in this workspace yet, §3/§4), no radial quick-actions, no minimap
//! fusion. This is real fuzzy file-jump plus a real fixed command list,
//! honestly scoped to what this crate can actually back today, the same
//! "name the gap, don't fake it" discipline `mode_toggle.rs`'s own doc
//! comment already applies to whole modes.

use std::path::{Path, PathBuf};

/// The real, fixed set of actions the palette can execute -- each backed
/// by an already-real keybinding elsewhere in this crate (Ctrl+S, Ctrl+Z/
/// Y, Ctrl+W, Ctrl+G, Ctrl+1/2/3); the palette is a second real way to
/// reach the same real functionality, not a new implementation of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    SaveFile,
    Undo,
    Redo,
    CloseTab,
    ToggleSidebar,
    SwitchToAgentMode,
    SwitchToEditorMode,
}

impl CommandId {
    pub const ALL: [CommandId; 7] = [
        CommandId::SaveFile,
        CommandId::Undo,
        CommandId::Redo,
        CommandId::CloseTab,
        CommandId::ToggleSidebar,
        CommandId::SwitchToAgentMode,
        CommandId::SwitchToEditorMode,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CommandId::SaveFile => "Save File",
            CommandId::Undo => "Undo",
            CommandId::Redo => "Redo",
            CommandId::CloseTab => "Close Tab",
            CommandId::ToggleSidebar => "Toggle Sidebar: File Tree / Source Control",
            CommandId::SwitchToAgentMode => "Switch to Agent Mode",
            CommandId::SwitchToEditorMode => "Switch to Editor Mode",
        }
    }
}

/// One entry the palette can show and select -- either a static command or
/// a real file under the current project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntry {
    Command(CommandId),
    File(PathBuf),
}

impl PaletteEntry {
    pub fn display_label(&self) -> String {
        match self {
            PaletteEntry::Command(id) => id.label().to_string(),
            PaletteEntry::File(path) => path.display().to_string(),
        }
    }
}

pub fn static_commands() -> Vec<PaletteEntry> {
    CommandId::ALL
        .iter()
        .map(|id| PaletteEntry::Command(*id))
        .collect()
}

/// Real, recursive (depth-bounded) file listing under `root`, skipping
/// hidden directories and the handful of dependency/build directory names
/// this workspace itself has (`target`, `node_modules`) -- a real, named
/// v1 simplification (not `.gitignore`-aware), matching `file_tree.rs`'s
/// own "re-read from disk every time, no caching" choice. A directory this
/// process can't read is silently skipped, same degrade-gracefully pattern
/// `file_tree.rs` already established.
pub fn collect_files(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_files_inner(root, 0, max_depth, &mut out);
    out
}

fn collect_files_inner(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            collect_files_inner(&path, depth + 1, max_depth, out);
        } else {
            out.push(path);
        }
    }
}

/// Real, simple case-insensitive subsequence fuzzy match: every character
/// of `query` must appear in `text` in order (not necessarily
/// contiguous). Returns a real match score (lower is better -- the total
/// gap between consecutive matched characters, so contiguous/early matches
/// rank first) or `None` if `query` isn't a subsequence of `text` at all.
/// An empty query matches everything with score 0, so opening the palette
/// with nothing typed yet shows the full, unranked list.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let text_lower: Vec<char> = text.to_lowercase().chars().collect();
    let mut score: i64 = 0;
    let mut search_from = 0usize;
    let mut last_match: Option<usize> = None;
    for q in query.to_lowercase().chars() {
        let found = (search_from..text_lower.len()).find(|&i| text_lower[i] == q)?;
        score += match last_match {
            Some(last) => (found - last - 1) as i64,
            None => found as i64,
        };
        last_match = Some(found);
        search_from = found + 1;
    }
    Some(score)
}

/// Filters `entries` to those whose display label fuzzy-matches `query`,
/// sorted best-match-first (lowest score first); Rust's sort is stable, so
/// ties keep `entries`' original relative order -- commands (listed first
/// by every caller in this crate) rank above equally-scored files.
pub fn filter_and_rank(query: &str, entries: &[PaletteEntry]) -> Vec<PaletteEntry> {
    let mut scored: Vec<(i64, &PaletteEntry)> = entries
        .iter()
        .filter_map(|entry| fuzzy_score(query, &entry.display_label()).map(|s| (s, entry)))
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, e)| e.clone()).collect()
}

/// Builds the palette's real display text for `TextState::set_modal_text`:
/// the typed query on its own first line, a blank separator, then one line
/// per ranked entry with the currently-selected one prefixed `"> "` and
/// every other `"  "` -- the same prefix-marker convention
/// `file_tree::build_tree_text` already established, reused here for a
/// selection cursor instead of expand state.
pub fn build_palette_text(query: &str, entries: &[PaletteEntry], selected: usize) -> String {
    let mut text = format!("Command Palette: {query}\n");
    if entries.is_empty() {
        text.push_str("\n  (no matches)");
        return text;
    }
    for (index, entry) in entries.iter().enumerate() {
        let marker = if index == selected { "> " } else { "  " };
        text.push('\n');
        text.push_str(marker);
        text.push_str(&entry.display_label());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything_with_score_zero() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn a_query_that_is_not_a_subsequence_does_not_match() {
        assert_eq!(fuzzy_score("xyz", "Save File"), None);
    }

    #[test]
    fn a_contiguous_early_match_scores_lower_than_a_scattered_one() {
        let contiguous = fuzzy_score("save", "Save File").unwrap();
        let scattered = fuzzy_score("sfl", "Save File").unwrap();
        assert!(contiguous < scattered);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(
            fuzzy_score("SAVE", "save file"),
            fuzzy_score("save", "save file")
        );
    }

    #[test]
    fn filter_and_rank_drops_non_matches_and_orders_best_match_first() {
        let entries = vec![
            PaletteEntry::Command(CommandId::SaveFile),
            PaletteEntry::Command(CommandId::Undo),
            PaletteEntry::File(PathBuf::from("main.rs")),
        ];
        let ranked = filter_and_rank("un", &entries);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0], PaletteEntry::Command(CommandId::Undo));
    }

    #[test]
    fn filter_and_rank_with_an_empty_query_keeps_original_order() {
        let entries = vec![
            PaletteEntry::Command(CommandId::SaveFile),
            PaletteEntry::Command(CommandId::Undo),
        ];
        let ranked = filter_and_rank("", &entries);
        assert_eq!(ranked, entries);
    }

    #[test]
    fn collect_files_lists_real_files_recursively_and_skips_target_and_hidden_dirs() {
        let root = std::env::temp_dir().join("spartan_palette_test_collect");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("src").join("lib.rs"), "").unwrap();
        std::fs::write(root.join("target").join("built.bin"), "").unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "").unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();

        let files = collect_files(&root, 8);
        let _ = std::fs::remove_dir_all(&root);

        assert!(files.contains(&root.join("src").join("lib.rs")));
        assert!(files.contains(&root.join("Cargo.toml")));
        assert!(!files.iter().any(|p| p.starts_with(root.join("target"))));
        assert!(!files.iter().any(|p| p.starts_with(root.join(".git"))));
    }

    #[test]
    fn build_palette_text_marks_only_the_selected_row() {
        let entries = vec![
            PaletteEntry::Command(CommandId::SaveFile),
            PaletteEntry::Command(CommandId::Undo),
        ];
        let text = build_palette_text("sa", &entries, 1);
        assert_eq!(text, "Command Palette: sa\n\n  Save File\n> Undo");
    }

    #[test]
    fn build_palette_text_with_no_matches_says_so() {
        let text = build_palette_text("zzz", &[], 0);
        assert_eq!(text, "Command Palette: zzz\n\n  (no matches)");
    }
}
