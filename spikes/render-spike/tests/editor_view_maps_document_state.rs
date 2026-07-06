use rand::SeedableRng;
use render_spike::editor_view::{EditEffect, EditorView};

/// Runs headless -- no wgpu device, no window, no display -- exercising only
/// the `Document` <-> render-input mapping logic (`EditorView`), per this
/// spike's plan. Everything GPU/winit-facing lives in the binary target and
/// isn't reachable from here.
#[test]
fn new_places_cursor_at_end_of_initial_text() {
    let editor = EditorView::new("abc");
    assert_eq!(editor.cursor, 3);
    assert_eq!(editor.text(), "abc");
}

#[test]
fn insert_at_cursor_appends_and_advances_cursor() {
    let mut editor = EditorView::new("ab");
    editor.insert_at_cursor("c");
    assert_eq!(editor.text(), "abc");
    assert_eq!(editor.cursor, 3);
}

#[test]
fn insert_at_cursor_advances_by_char_count_not_byte_count() {
    // "é" is 1 char but 2 UTF-8 bytes -- the multi-byte-boundary bug class
    // §48 already found once in `spartan-buffer` itself. Advancing the
    // cursor by byte length here would silently desync it from the
    // document's own char-indexed API on the very next edit.
    let mut editor = EditorView::new("caf");
    editor.insert_at_cursor("é");
    assert_eq!(editor.text(), "café");
    assert_eq!(editor.cursor, 4);
}

#[test]
fn backspace_removes_preceding_char_and_moves_cursor_back() {
    let mut editor = EditorView::new("abc");
    editor.backspace();
    assert_eq!(editor.text(), "ab");
    assert_eq!(editor.cursor, 2);
}

#[test]
fn backspace_at_start_of_document_is_a_no_op() {
    let mut editor = EditorView::new("abc");
    editor.cursor = 0;
    editor.backspace();
    assert_eq!(editor.text(), "abc");
    assert_eq!(editor.cursor, 0);
}

#[test]
fn cursor_line_col_on_first_line() {
    let mut editor = EditorView::new("hello\nworld");
    editor.cursor = 3;
    assert_eq!(editor.cursor_line_col(), (0, 3));
}

#[test]
fn cursor_line_col_after_a_newline() {
    let editor = EditorView::new("hello\nworld");
    // cursor defaults to end-of-document: line 1 ("world"), column 5.
    assert_eq!(editor.cursor_line_col(), (1, 5));
}

#[test]
fn cursor_line_col_on_the_phantom_trailing_line_after_a_final_newline() {
    // The exact mismatch Step 5 found by running this, not by inspection:
    // `Document` (ropey) counts one more, empty line after a file's final
    // "\n" than cosmic-text's `Buffer` lays out (see
    // `text::TextState::cursor_pixel_pos`'s doc comment). This test locks in
    // `EditorView`'s half of that behavior -- cursor at end-of-document on a
    // trailing-newline file reports line 3 (one past the 3 real lines),
    // column 0 -- as a regression guard, since `cursor_pixel_pos` depends on
    // exactly this shape to trigger its fallback.
    let editor = EditorView::new("fn main() {\n    println!(\"hi\");\n}\n");
    assert_eq!(editor.cursor_line_col(), (3, 0));
}

#[test]
fn insert_random_keeps_document_valid_and_grows_it_by_one_char_per_call() {
    let mut editor = EditorView::new("fn main() {}\n");
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let start_len = editor.document.len_chars();

    for i in 1..=50 {
        editor.insert_random(&mut rng, "x");
        assert_eq!(editor.document.len_chars(), start_len + i);
    }
}

#[test]
fn insert_random_advances_cursor_only_when_insertion_is_at_or_before_it() {
    let mut editor = EditorView::new("abcdefghij");
    editor.cursor = 5;
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);

    for _ in 0..20 {
        let cursor_before = editor.cursor;
        editor.insert_random(&mut rng, "x");
        // The cursor never moves backward and advances by at most one char
        // per insertion, regardless of where in the document that
        // insertion actually landed.
        assert!(editor.cursor == cursor_before || editor.cursor == cursor_before + 1);
    }
}

// -- EditEffect: the damage-region increment's line-vs-structural
// classification (Track A). Getting this wrong in either direction is a
// real correctness bug, not just a missed optimization: reporting
// `Structural` for a same-line edit only wastes a reshape, but reporting
// `Line` for a structural edit would leave stale/wrong content on screen
// (see `text::TextState::set_line_text`'s doc comment).

#[test]
fn insert_at_cursor_without_newline_is_a_line_effect_on_the_cursors_line() {
    let mut editor = EditorView::new("hello\nworld");
    editor.cursor = 3; // mid "hello", line 0
    assert_eq!(editor.insert_at_cursor("X"), EditEffect::Line(0));
}

#[test]
fn insert_at_cursor_with_a_newline_is_structural() {
    let mut editor = EditorView::new("hello");
    editor.cursor = 5;
    assert_eq!(editor.insert_at_cursor("\n"), EditEffect::Structural);
    assert_eq!(editor.document.len_lines(), 2);
}

#[test]
fn insert_at_cursor_when_document_insert_fails_reports_none() {
    let mut editor = EditorView::new("abc");
    editor.cursor = 999; // out of bounds -- Document::insert will error
    assert_eq!(editor.insert_at_cursor("x"), EditEffect::None);
}

#[test]
fn backspace_on_an_ordinary_character_is_a_line_effect() {
    let mut editor = EditorView::new("hello\nworld");
    editor.cursor = 11; // end of "world", line 1
    assert_eq!(editor.backspace(), EditEffect::Line(1));
    assert_eq!(editor.text(), "hello\nworl");
}

#[test]
fn backspace_at_the_start_of_a_line_merges_lines_and_is_structural() {
    let mut editor = EditorView::new("hello\nworld");
    editor.cursor = 6; // right after the "\n", start of "world"
    assert_eq!(editor.backspace(), EditEffect::Structural);
    assert_eq!(editor.text(), "helloworld");
    assert_eq!(editor.document.len_lines(), 1);
}

#[test]
fn backspace_at_start_of_document_reports_none() {
    let mut editor = EditorView::new("abc");
    editor.cursor = 0;
    assert_eq!(editor.backspace(), EditEffect::None);
}

#[test]
fn insert_after_a_trailing_newline_reports_a_line_index_a_fresh_cosmic_text_rebuild_would_not_yet_have(
) {
    // Regression test for a real bug found by actually running the
    // program, not by inspection (see `text::TextState::line_count`'s doc
    // comment for the full story): right after Enter creates a trailing
    // newline, ropey immediately considers the cursor to be on a new real
    // line -- but a fresh cosmic-text rebuild of that same text wouldn't
    // yet contain that line, since cosmic-text never synthesizes a phantom
    // trailing line (the same mismatch `cursor_pixel_pos` already handles
    // for cursor rendering). This test locks in this crate's half of the
    // mismatch: the line index reported for the character typed
    // immediately after a fresh newline is exactly `len_lines() - 1`
    // measured right after that newline. Callers (see `main.rs`'s
    // `apply_edit_effect`) must never call `TextState::set_line_text` with
    // this index without first checking `line_i < TextState::line_count()`
    // -- doing so silently drops the typed character.
    let mut editor = EditorView::new("ab");
    editor.cursor = 2;
    assert_eq!(editor.insert_at_cursor("\n"), EditEffect::Structural);
    let lines_after_newline = editor.document.len_lines();
    let effect = editor.insert_at_cursor("c");
    assert_eq!(effect, EditEffect::Line(lines_after_newline - 1));
}

#[test]
fn insert_random_without_newline_is_always_a_line_effect() {
    let mut editor = EditorView::new("fn main() {\n    body();\n}\n");
    let mut rng = rand::rngs::StdRng::seed_from_u64(99);
    for _ in 0..30 {
        let effect = editor.insert_random(&mut rng, "x");
        assert!(matches!(effect, EditEffect::Line(_)));
    }
}
