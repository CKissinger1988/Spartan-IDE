use rand::SeedableRng;
use render_spike::editor_view::EditorView;

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
