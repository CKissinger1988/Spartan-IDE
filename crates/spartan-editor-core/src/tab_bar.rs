//! Real, pure tab-bar layout logic (§75.21) -- builds the tab bar's single
//! display string and tracks each open tab's real char-range within it, so
//! a click can be resolved to "which tab" (and "was it the close button")
//! via the same real cosmic-text hit-testing `TextState::hit_test` already
//! uses for the main editor -- rather than a second, separate,
//! pixel-guessing geometry model. No GPU dependency, unlike the rendering
//! itself (`main.rs`, `text.rs`), so headlessly testable here.

use std::ops::Range;

/// One open tab's real char-range within the tab bar's built string.
/// `tab_range` covers the whole clickable tab (switches to it);
/// `close_range` is the narrower "×" sub-range within it (closes it
/// instead) and is always contained within `tab_range`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabHit {
    pub file_index: usize,
    pub tab_range: Range<usize>,
    pub close_range: Range<usize>,
}

/// Builds the tab bar's single-line display string from each open file's
/// `(display_name, dirty)`, plus one `TabHit` per tab recording exactly
/// where it (and its close button) landed in that string. Tabs are
/// separated by `"│"`; each tab reads ` name[ *] × ` (the `*` only present
/// when dirty, matching `window_title`'s own convention). Char-counted
/// throughout, not byte-counted, matching this crate's convention
/// everywhere else (`EditorView::cursor_line_col`, `TextState::hit_test`).
pub fn build_tab_bar_text(files: &[(String, bool)]) -> (String, Vec<TabHit>) {
    let mut text = String::new();
    let mut hits = Vec::with_capacity(files.len());
    let mut pos = 0usize;
    for (file_index, (name, dirty)) in files.iter().enumerate() {
        if file_index > 0 {
            text.push('│');
            pos += 1;
        }
        let tab_start = pos;
        text.push(' ');
        pos += 1;
        text.push_str(name);
        pos += name.chars().count();
        if *dirty {
            text.push_str(" *");
            pos += 2;
        }
        text.push(' ');
        pos += 1;
        let close_start = pos;
        text.push('×');
        pos += 1;
        let close_end = pos;
        text.push(' ');
        pos += 1;
        let tab_end = pos;
        hits.push(TabHit {
            file_index,
            tab_range: tab_start..tab_end,
            close_range: close_start..close_end,
        });
    }
    (text, hits)
}

/// Resolves a real char index (from hit-testing the tab bar's own rendered
/// text) to `(file_index, is_close_button)`, or `None` if the click landed
/// between tabs / past the end of the last one.
pub fn hit_test_tab_bar(hits: &[TabHit], char_index: usize) -> Option<(usize, bool)> {
    hits.iter().find_map(|hit| {
        if hit.tab_range.contains(&char_index) {
            Some((hit.file_index, hit.close_range.contains(&char_index)))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tab_has_no_leading_separator() {
        let (text, hits) = build_tab_bar_text(&[("main.rs".to_string(), false)]);
        assert_eq!(text, " main.rs × ");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file_index, 0);
        assert_eq!(hits[0].tab_range, 0..text.chars().count());
    }

    #[test]
    fn dirty_tab_includes_the_star_marker() {
        let (text, _hits) = build_tab_bar_text(&[("main.rs".to_string(), true)]);
        assert_eq!(text, " main.rs * × ");
    }

    #[test]
    fn two_tabs_are_separated_and_each_range_is_correct() {
        let (text, hits) =
            build_tab_bar_text(&[("a.rs".to_string(), false), ("b.rs".to_string(), false)]);
        assert_eq!(text, " a.rs × │ b.rs × ");
        assert_eq!(hits.len(), 2);
        let first = &text.chars().collect::<Vec<_>>()[hits[0].tab_range.clone()];
        assert_eq!(first.iter().collect::<String>(), " a.rs × ");
        let second = &text.chars().collect::<Vec<_>>()[hits[1].tab_range.clone()];
        assert_eq!(second.iter().collect::<String>(), " b.rs × ");
    }

    #[test]
    fn close_range_is_contained_within_tab_range() {
        let (_text, hits) = build_tab_bar_text(&[("main.rs".to_string(), false)]);
        let hit = &hits[0];
        assert!(hit.tab_range.start <= hit.close_range.start);
        assert!(hit.close_range.end <= hit.tab_range.end);
    }

    #[test]
    fn hit_test_tab_bar_finds_the_containing_tab() {
        let (_text, hits) =
            build_tab_bar_text(&[("a.rs".to_string(), false), ("b.rs".to_string(), false)]);
        // Middle of the second tab's name.
        let second_tab_char = hits[1].tab_range.start + 2;
        assert_eq!(hit_test_tab_bar(&hits, second_tab_char), Some((1, false)));
    }

    #[test]
    fn hit_test_tab_bar_detects_the_close_button() {
        let (_text, hits) = build_tab_bar_text(&[("main.rs".to_string(), false)]);
        let close_char = hits[0].close_range.start;
        assert_eq!(hit_test_tab_bar(&hits, close_char), Some((0, true)));
    }

    #[test]
    fn hit_test_tab_bar_returns_none_past_the_last_tab() {
        let (text, hits) = build_tab_bar_text(&[("main.rs".to_string(), false)]);
        assert_eq!(hit_test_tab_bar(&hits, text.chars().count() + 5), None);
    }

    #[test]
    fn build_tab_bar_text_with_no_open_files_is_empty() {
        let (text, hits) = build_tab_bar_text(&[]);
        assert_eq!(text, "");
        assert!(hits.is_empty());
    }
}
