use rand::Rng;
use spartan_buffer::Document;

/// Owns the real `spartan-buffer::Document` this spike edits and renders --
/// the same buffer model the rest of the project already uses, not a
/// throwaway `String` invented for this spike -- plus the one piece of
/// state the buffer itself deliberately doesn't own: where the caret is.
pub struct EditorView {
    pub document: Document,
    pub cursor: usize,
}

impl EditorView {
    pub fn new(initial_text: &str) -> Self {
        let document = Document::new(initial_text);
        let cursor = document.len_chars();
        Self { document, cursor }
    }

    pub fn text(&self) -> String {
        self.document.text()
    }

    /// Inserts at the cursor and advances it past the inserted text.
    pub fn insert_at_cursor(&mut self, text: &str) {
        if self.document.insert(self.cursor, text).is_ok() {
            self.cursor += text.chars().count();
        }
    }

    /// Deletes the character immediately before the cursor (Backspace).
    pub fn backspace(&mut self) {
        if self.cursor > 0 && self.document.delete(self.cursor - 1..self.cursor).is_ok() {
            self.cursor -= 1;
        }
    }

    /// Inserts a single piece of text at a uniformly random position, for the
    /// internally-scripted latency benchmark (Steps 6/8) -- deliberately
    /// bypasses `self.cursor` as the insertion point, since a benchmark
    /// driver isn't a real caret and rope-spike's own `bench_rope_typing`
    /// (§47.1) measures the same way: random-position inserts, not
    /// append-only ones, so a large document's edit cost isn't
    /// under-measured by only ever touching its end.
    pub fn insert_random(&mut self, rng: &mut impl Rng, text: &str) {
        let pos = rng.gen_range(0..=self.document.len_chars());
        if self.document.insert(pos, text).is_ok() && pos <= self.cursor {
            self.cursor += text.chars().count();
        }
    }

    /// Cursor's (line, column-in-chars) position, for rendering (Step 5).
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.document.char_to_line(self.cursor).unwrap_or(0);
        let line_start = self.document.line_to_char(line).unwrap_or(0);
        (line, self.cursor - line_start)
    }
}
