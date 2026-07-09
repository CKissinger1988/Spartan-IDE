use spartan_buffer::Document;
use spartan_editor_core::editor_view::EditorView;
use spartan_editor_core::language::detect_language_for_file;
use spartan_editor_core::viewport::{to_doc_line, to_local_line, windowed_text, Viewport};
use std::path::Path;

// Runs headless -- no wgpu device, no window, no display -- exercising only
// the `Viewport`/`windowed_text`/`to_local_line` slicing logic and the
// `spartan-languages` lookup wiring. Everything GPU/winit-facing lives in
// the binary target and isn't reachable from here, matching the pattern
// `spikes/render-spike/tests/editor_view_maps_document_state.rs` already
// established.

fn five_line_doc() -> Document {
    Document::new("line0\nline1\nline2\nline3\nline4\n")
}

#[test]
fn new_viewport_starts_scrolled_to_the_top() {
    let viewport = Viewport::new(3);
    assert_eq!(viewport.scroll_line, 0);
    assert_eq!(viewport.visible_lines, 3);
}

#[test]
fn visible_range_is_bounded_by_visible_lines_when_the_document_is_larger() {
    let doc = five_line_doc();
    let viewport = Viewport::new(3);
    assert_eq!(viewport.visible_range(doc.len_lines()), 0..3);
}

#[test]
fn visible_range_shrinks_near_the_end_of_a_document_shorter_than_visible_lines() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(3);
    viewport.scroll_line = 4;
    // Only line 4 (of 6, counting ropey's phantom trailing line) is left --
    // `visible_range` must clamp to `doc_len_lines`, not overrun it.
    assert_eq!(viewport.visible_range(doc.len_lines()), 4..doc.len_lines());
}

#[test]
fn windowed_text_extracts_only_the_visible_slice() {
    let doc = five_line_doc();
    let viewport = Viewport::new(2);
    assert_eq!(windowed_text(&doc, &viewport), "line0\nline1\n");
}

#[test]
fn windowed_text_after_scrolling_reflects_the_new_window() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 2;
    assert_eq!(windowed_text(&doc, &viewport), "line2\nline3\n");
}

#[test]
fn scroll_by_moves_forward_and_reports_a_real_change() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    let changed = viewport.scroll_by(2, doc.len_lines());
    assert!(changed);
    assert_eq!(viewport.scroll_line, 2);
}

#[test]
fn scroll_by_clamps_at_the_last_valid_line_and_reports_no_change_once_there() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    let max_scroll = doc.len_lines() - 1;

    let first = viewport.scroll_by(1000, doc.len_lines());
    assert!(first);
    assert_eq!(viewport.scroll_line, max_scroll);

    // Already clamped at the edge -- a second identical scroll must report
    // "no change" so callers (main.rs) can skip a wasted reshape.
    let second = viewport.scroll_by(1000, doc.len_lines());
    assert!(!second);
    assert_eq!(viewport.scroll_line, max_scroll);
}

#[test]
fn scroll_by_negative_delta_clamps_at_zero_rather_than_wrapping() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 1;
    let changed = viewport.scroll_by(-100, doc.len_lines());
    assert!(changed);
    assert_eq!(viewport.scroll_line, 0);
}

#[test]
fn to_local_line_translates_a_visible_document_line() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 2;
    assert_eq!(to_local_line(3, &viewport, doc.len_lines()), Some(1));
}

#[test]
fn to_local_line_returns_none_for_a_line_outside_the_current_window() {
    let doc = five_line_doc();
    let viewport = Viewport::new(2); // shows lines 0..2
    assert_eq!(to_local_line(4, &viewport, doc.len_lines()), None);
}

#[test]
fn to_local_line_returns_none_for_a_line_scrolled_above_the_window() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 3; // shows lines 3..5
    assert_eq!(to_local_line(0, &viewport, doc.len_lines()), None);
}

#[test]
fn to_doc_line_is_the_inverse_of_to_local_line() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 2;
    let local = to_local_line(3, &viewport, doc.len_lines()).unwrap();
    assert_eq!(to_doc_line(local, &viewport), 3);
}

#[test]
fn to_doc_line_at_zero_scroll_is_the_identity() {
    let viewport = Viewport::new(3);
    assert_eq!(to_doc_line(2, &viewport), 2);
}

#[test]
fn set_cursor_to_line_col_places_the_cursor_at_the_requested_position() {
    let mut editor = EditorView::new("line0\nline1\nline2\n");
    editor.set_cursor_to_line_col(1, 3);
    assert_eq!(editor.cursor_line_col(), (1, 3));
}

#[test]
fn set_cursor_to_line_col_clamps_a_column_past_end_of_a_short_line() {
    let mut editor = EditorView::new("hi\nlonger line\n");
    // Line 0 ("hi") is only 2 chars -- clicking far past its end should
    // land at end-of-line, not panic or wrap into the next line.
    editor.set_cursor_to_line_col(0, 50);
    assert_eq!(editor.cursor_line_col(), (0, 2));
}

#[test]
fn set_cursor_to_line_col_clamps_a_line_past_end_of_document() {
    let mut editor = EditorView::new("only one line\n");
    editor.set_cursor_to_line_col(99, 0);
    let (line, _) = editor.cursor_line_col();
    assert_eq!(line, editor.document.len_lines() - 1);
}

#[test]
fn contains_line_agrees_with_to_local_line() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 2;
    for line in 0..doc.len_lines() {
        assert_eq!(
            viewport.contains_line(line, doc.len_lines()),
            to_local_line(line, &viewport, doc.len_lines()).is_some()
        );
    }
}

#[test]
fn ensure_visible_is_a_no_op_when_the_line_is_already_visible() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2); // shows lines 0..2
    let changed = viewport.ensure_visible(1, doc.len_lines());
    assert!(!changed);
    assert_eq!(viewport.scroll_line, 0);
}

#[test]
fn ensure_visible_scrolls_up_when_the_line_is_above_the_window() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    viewport.scroll_line = 4;
    let changed = viewport.ensure_visible(1, doc.len_lines());
    assert!(changed);
    // The target line becomes the new first visible line.
    assert_eq!(viewport.scroll_line, 1);
}

#[test]
fn ensure_visible_scrolls_down_so_the_line_becomes_the_last_visible_one() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2); // shows lines 0..2
    let changed = viewport.ensure_visible(3, doc.len_lines());
    assert!(changed);
    // visible_lines=2, so scroll_line=2 shows lines 2..4 -- line 3 is last.
    assert_eq!(viewport.scroll_line, 2);
    assert!(viewport.contains_line(3, doc.len_lines()));
}

#[test]
fn ensure_visible_scrolling_down_to_the_documents_actual_last_line_stays_in_bounds() {
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    let last_line = doc.len_lines() - 1;
    let changed = viewport.ensure_visible(last_line, doc.len_lines());
    assert!(changed);
    assert!(viewport.contains_line(last_line, doc.len_lines()));
    assert!(viewport.scroll_line <= last_line);
}

#[test]
fn ensure_visible_clamps_even_for_a_doc_line_past_the_documents_end() {
    // A real cursor position should never pass an out-of-range line, but
    // the clamp is kept as a defensive guard anyway -- this locks in that
    // it degrades to the last valid scroll position rather than producing
    // a `scroll_line` no `visible_range` call could ever recover from.
    let doc = five_line_doc();
    let mut viewport = Viewport::new(2);
    let out_of_range = doc.len_lines() + 10;
    let changed = viewport.ensure_visible(out_of_range, doc.len_lines());
    assert!(changed);
    assert_eq!(viewport.scroll_line, doc.len_lines() - 1);
}

// -- spartan-languages integration: the first real, if narrow, combination
// of the buffer/renderer crate with the language registry (§75.3's named
// next gap).

#[test]
fn detects_rust_for_a_real_source_file_in_this_workspace() {
    let profile = detect_language_for_file(Path::new("src/editor_view.rs"))
        .expect("a .rs file under this crate's own source tree must resolve to a language profile");
    assert_eq!(profile.id, "rust");
    assert!(!profile.tree_sitter_grammar.is_empty());
}

#[test]
fn returns_none_for_a_file_extension_no_curated_profile_covers() {
    assert!(detect_language_for_file(Path::new("notes.xyz123")).is_none());
}
