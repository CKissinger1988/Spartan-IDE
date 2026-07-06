use rand::Rng;
use spartan_buffer::Document;

/// What kind of re-render a completed edit requires -- the type this
/// spike's damage-region increment (Track A) hinges on. `Line` means only
/// that one line's cached glyphon shape/layout needs invalidating (see
/// `text::TextState::set_line_text`); `Structural` means the edit changed
/// the document's line count (a newline inserted or removed), which needs a
/// full-document reshape since cosmic-text has no public API for cheap line
/// insert/delete. `None` means nothing actually changed (e.g. backspace at
/// the very start of an empty document).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditEffect {
    None,
    Line(usize),
    Structural,
}

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
    pub fn insert_at_cursor(&mut self, text: &str) -> EditEffect {
        let line_before = self.document.char_to_line(self.cursor).ok();
        if self.document.insert(self.cursor, text).is_ok() {
            self.cursor += text.chars().count();
            if text.contains('\n') {
                EditEffect::Structural
            } else {
                line_before.map_or(EditEffect::Structural, EditEffect::Line)
            }
        } else {
            EditEffect::None
        }
    }

    /// Deletes the character immediately before the cursor (Backspace).
    pub fn backspace(&mut self) -> EditEffect {
        if self.cursor == 0 {
            return EditEffect::None;
        }
        let line_before = self.document.char_to_line(self.cursor).ok();
        // If the cursor sits at the very start of its line, the character
        // being removed is the previous line's terminating "\n" -- deleting
        // it merges two lines into one, which is structural, not a
        // same-line edit.
        let at_line_start =
            line_before.and_then(|l| self.document.line_to_char(l).ok()) == Some(self.cursor);
        if self.document.delete(self.cursor - 1..self.cursor).is_ok() {
            self.cursor -= 1;
            if at_line_start {
                EditEffect::Structural
            } else {
                line_before.map_or(EditEffect::Structural, EditEffect::Line)
            }
        } else {
            EditEffect::None
        }
    }

    /// Inserts a single piece of text at a uniformly random position, for the
    /// internally-scripted latency benchmark (Steps 6/8) -- deliberately
    /// bypasses `self.cursor` as the insertion point, since a benchmark
    /// driver isn't a real caret and rope-spike's own `bench_rope_typing`
    /// (§47.1) measures the same way: random-position inserts, not
    /// append-only ones, so a large document's edit cost isn't
    /// under-measured by only ever touching its end.
    pub fn insert_random(&mut self, rng: &mut impl Rng, text: &str) -> EditEffect {
        let pos = rng.gen_range(0..=self.document.len_chars());
        let line_before = self.document.char_to_line(pos).ok();
        if self.document.insert(pos, text).is_ok() {
            if pos <= self.cursor {
                self.cursor += text.chars().count();
            }
            if text.contains('\n') {
                EditEffect::Structural
            } else {
                line_before.map_or(EditEffect::Structural, EditEffect::Line)
            }
        } else {
            EditEffect::None
        }
    }

    /// Cursor's (line, column-in-chars) position, for rendering (Step 5).
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.document.char_to_line(self.cursor).unwrap_or(0);
        let line_start = self.document.line_to_char(line).unwrap_or(0);
        (line, self.cursor - line_start)
    }
}
