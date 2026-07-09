//! Real Source Control panel display logic (§56.1, task #7) -- pure,
//! no-GPU layout built from a real `spartan_git::StatusEntry` list, the
//! same "pure logic, no GPU dependency" split `file_tree.rs`/`tab_bar.rs`
//! already established. Re-reads real git status from scratch on every
//! `build_rows()` call (via the caller re-querying `spartan_git::GitRepo`)
//! -- the same deliberate "no caching" v1 simplification `file_tree.rs`
//! already named for the filesystem side, now true for git status too.

use spartan_git::{FileStatus, StatusEntry};
use std::path::PathBuf;

/// One visible row in the panel -- either a section header (not
/// clickable) or a real file entry (clickable: stages/unstages
/// depending on which section it's in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelRow {
    Header(String),
    File { path: PathBuf, staged: bool },
}

/// Splits a real status list into "Staged Changes" and "Changes"
/// sections. A file with both a staged *and* an unstaged component (git's
/// real, valid "staged some of it, then edited further" state) appears in
/// both sections, once per section -- clicking the staged-section row
/// unstages only the staged half; clicking the unstaged-section row
/// stages the file's current working-tree content on top.
pub fn build_rows(entries: &[StatusEntry]) -> Vec<PanelRow> {
    let staged: Vec<&StatusEntry> = entries.iter().filter(|e| e.staged.is_some()).collect();
    let unstaged: Vec<&StatusEntry> = entries
        .iter()
        .filter(|e| e.unstaged.is_some() || e.conflicted)
        .collect();

    let mut rows = Vec::new();
    rows.push(PanelRow::Header(format!(
        "Staged Changes ({})",
        staged.len()
    )));
    for entry in &staged {
        rows.push(PanelRow::File {
            path: entry.path.clone(),
            staged: true,
        });
    }
    rows.push(PanelRow::Header(format!("Changes ({})", unstaged.len())));
    for entry in &unstaged {
        rows.push(PanelRow::File {
            path: entry.path.clone(),
            staged: false,
        });
    }
    rows
}

fn glyph_for(status: FileStatus) -> &'static str {
    match status {
        FileStatus::Added => "A ",
        FileStatus::Modified => "M ",
        FileStatus::Deleted => "D ",
        FileStatus::Renamed => "R ",
        FileStatus::TypeChanged => "T ",
    }
}

/// Real display text -- one line per row. A file row's glyph is looked
/// up from `entries` directly (rather than carried on `PanelRow` itself)
/// so `PanelRow` stays a simple, hit-testable shape.
pub fn build_panel_text(rows: &[PanelRow], entries: &[StatusEntry]) -> String {
    rows.iter()
        .map(|row| match row {
            PanelRow::Header(text) => text.clone(),
            PanelRow::File { path, staged } => {
                let entry = entries.iter().find(|e| &e.path == path);
                let status = entry.and_then(|e| if *staged { e.staged } else { e.unstaged });
                let glyph = if entry.is_some_and(|e| e.conflicted) {
                    "! "
                } else {
                    status.map(glyph_for).unwrap_or("? ")
                };
                format!("  {glyph}{}", path.display())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, staged: Option<FileStatus>, unstaged: Option<FileStatus>) -> StatusEntry {
        StatusEntry {
            path: PathBuf::from(path),
            staged,
            unstaged,
            conflicted: false,
        }
    }

    #[test]
    fn build_rows_splits_staged_and_unstaged_into_separate_sections() {
        let entries = vec![
            entry("a.rs", Some(FileStatus::Added), None),
            entry("b.rs", None, Some(FileStatus::Modified)),
        ];
        let rows = build_rows(&entries);
        assert_eq!(
            rows,
            vec![
                PanelRow::Header("Staged Changes (1)".to_string()),
                PanelRow::File {
                    path: PathBuf::from("a.rs"),
                    staged: true
                },
                PanelRow::Header("Changes (1)".to_string()),
                PanelRow::File {
                    path: PathBuf::from("b.rs"),
                    staged: false
                },
            ]
        );
    }

    #[test]
    fn a_file_staged_and_further_modified_appears_in_both_sections() {
        let entries = vec![entry(
            "c.rs",
            Some(FileStatus::Added),
            Some(FileStatus::Modified),
        )];
        let rows = build_rows(&entries);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[1],
            PanelRow::File {
                path: PathBuf::from("c.rs"),
                staged: true
            }
        );
        assert_eq!(
            rows[3],
            PanelRow::File {
                path: PathBuf::from("c.rs"),
                staged: false
            }
        );
    }

    #[test]
    fn empty_status_still_produces_both_empty_section_headers() {
        let rows = build_rows(&[]);
        assert_eq!(
            rows,
            vec![
                PanelRow::Header("Staged Changes (0)".to_string()),
                PanelRow::Header("Changes (0)".to_string()),
            ]
        );
    }

    #[test]
    fn build_panel_text_uses_the_expected_glyphs() {
        let entries = vec![
            entry("a.rs", Some(FileStatus::Added), None),
            entry("b.rs", None, Some(FileStatus::Deleted)),
        ];
        let rows = build_rows(&entries);
        let text = build_panel_text(&rows, &entries);
        assert_eq!(text, "Staged Changes (1)\n  A a.rs\nChanges (1)\n  D b.rs");
    }

    #[test]
    fn a_conflicted_file_uses_the_conflict_glyph_regardless_of_status() {
        let entries = vec![StatusEntry {
            path: PathBuf::from("c.rs"),
            staged: None,
            unstaged: Some(FileStatus::Modified),
            conflicted: true,
        }];
        let rows = build_rows(&entries);
        let text = build_panel_text(&rows, &entries);
        assert!(text.contains("! c.rs"));
    }
}
