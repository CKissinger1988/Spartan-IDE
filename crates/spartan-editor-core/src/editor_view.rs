use rand::Rng;
use spartan_buffer::Document;

/// What kind of re-render a completed edit requires. Promoted from
/// `spikes/render-spike/src/editor_view.rs` (§39.1, §47.9-§47.10) verbatim
/// -- this classification is document-absolute and viewport-agnostic by
/// design (the viewport translates a `Line(doc_line_i)` into a window-local
/// index, or discards it entirely if the edit happened off-screen -- see
/// `viewport::to_local_line`), so it needed no changes to become real
/// product code.
///
/// `Line` means only that one line's cached glyphon shape/layout needs
/// invalidating; `Structural` means the edit changed the document's line
/// count (a newline inserted or removed), which needs a full (windowed)
/// reshape since cosmic-text has no public API for cheap line insert/delete.
/// `None` means nothing actually changed (e.g. backspace at the very start
/// of an empty document).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditEffect {
    None,
    Line(usize),
    Structural,
}

/// Owns the real `spartan-buffer::Document` this crate edits and renders --
/// the same buffer model the rest of the project already uses -- plus the
/// one piece of state the buffer itself deliberately doesn't own: where the
/// caret is. Both `document` and `cursor` are in document-absolute
/// coordinates; the viewport (`viewport.rs`) is a purely rendering-side
/// concern layered on top.
pub struct EditorView {
    pub document: Document,
    pub cursor: usize,
    /// Document-absolute char index the active selection is anchored at, if
    /// any (§75.18) -- `self.cursor` is always the *other*, moving end.
    /// `Some(anchor) == Some(cursor)` (a real click with no drag, or a
    /// selection collapsed back to zero width) is treated as "no selection"
    /// by `selection_range()`, not a real empty one, matching how a mouse
    /// click naturally arms an anchor before any actual drag has happened.
    pub selection_anchor: Option<usize>,
}

impl EditorView {
    /// One deliberate deviation from `render-spike`'s version: the cursor
    /// starts at document position 0 (top of file), not `len_chars()` (end
    /// of file). `render-spike` started at the end purely for its own demo
    /// convenience (append-and-watch-it-render); a real editor opening a
    /// file should show the cursor at the top, both because that's
    /// conventional and because it starts inside the viewport's initial
    /// visible range (`Viewport::new` also defaults `scroll_line` to 0),
    /// rather than off-screen at the bottom of a large file.
    pub fn new(initial_text: &str) -> Self {
        let document = Document::new(initial_text);
        let cursor = 0;
        Self {
            document,
            cursor,
            selection_anchor: None,
        }
    }

    pub fn text(&self) -> String {
        self.document.text()
    }

    /// Real, document-absolute `[start, end)` selection range (§75.18),
    /// normalized (`start <= end`) regardless of which direction the user
    /// dragged/extended from -- `None` if there's no active selection or
    /// the anchor and cursor coincide.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        if anchor == self.cursor {
            return None;
        }
        Some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    /// Arms a new selection anchored at the cursor's current position, if
    /// one isn't already active -- used by Shift+Arrow so extending an
    /// already-active selection doesn't silently reset its anchor back to
    /// wherever the cursor happens to be *now*.
    pub fn start_selection_if_needed(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
    }

    /// Deletes the active selection (if any) and moves the cursor to its
    /// start, clearing the anchor. `EditEffect::None` if there's no active
    /// selection to delete.
    pub fn delete_selection(&mut self) -> EditEffect {
        let Some((start, end)) = self.selection_range() else {
            return EditEffect::None;
        };
        let line_start = self.document.char_to_line(start).ok();
        let line_end = self.document.char_to_line(end).ok();
        let structural = line_start != line_end;
        if self.document.delete(start..end).is_ok() {
            self.cursor = start;
            self.selection_anchor = None;
            if structural {
                EditEffect::Structural
            } else {
                line_start.map_or(EditEffect::Structural, EditEffect::Line)
            }
        } else {
            EditEffect::None
        }
    }

    /// Inserts at the cursor and advances it past the inserted text. If a
    /// selection is active, it's real "typing over a selection" behavior
    /// (§75.18): the selection is deleted first and the new text takes its
    /// place, rather than being inserted alongside it. Replacing a
    /// selection always reports `Structural` regardless of what the
    /// deletion/insertion would individually have reported -- a real,
    /// deliberate simplification (a full windowed reshape is always
    /// correct, just not maximally cheap) rather than reasoning through
    /// every combination of the two effects.
    pub fn insert_at_cursor(&mut self, text: &str) -> EditEffect {
        let had_selection = self.selection_range().is_some();
        if had_selection {
            self.delete_selection();
        }
        let line_before = self.document.char_to_line(self.cursor).ok();
        if self.document.insert(self.cursor, text).is_ok() {
            self.cursor += text.chars().count();
            if had_selection || text.contains('\n') {
                EditEffect::Structural
            } else {
                line_before.map_or(EditEffect::Structural, EditEffect::Line)
            }
        } else {
            EditEffect::None
        }
    }

    /// Deletes the character immediately before the cursor (Backspace) --
    /// or, if a selection is active, deletes exactly the selection instead
    /// of the selection *plus* one more character before it, matching
    /// conventional editor behavior.
    pub fn backspace(&mut self) -> EditEffect {
        if self.selection_range().is_some() {
            return self.delete_selection();
        }
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

    /// Inserts a single piece of text at a uniformly random position, for
    /// the internally-scripted latency benchmark -- deliberately bypasses
    /// `self.cursor` as the insertion point, since a benchmark driver isn't
    /// a real caret and rope-spike's own `bench_rope_typing` (§47.1)
    /// measures the same way: random-position inserts, not append-only
    /// ones, so a large document's edit cost isn't under-measured by only
    /// ever touching its end.
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

    /// Cursor's (line, column-in-chars) position, in document-absolute
    /// coordinates -- callers rendering through a `Viewport` must translate
    /// the line via `viewport::to_local_line` before using it for anything
    /// glyphon-facing.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.document.char_to_line(self.cursor).unwrap_or(0);
        let line_start = self.document.line_to_char(line).unwrap_or(0);
        (line, self.cursor - line_start)
    }

    /// Sets the cursor to a document-absolute `(line, column-in-chars)`
    /// position -- the inverse of `cursor_line_col`, used by mouse
    /// click-to-position (`TextState::hit_test` + `viewport::to_doc_line`
    /// produce the position this takes). Both `line` and `col_chars` are
    /// clamped rather than trusted: `line` to the document's last real line
    /// (a click below the last line of a short file should still land
    /// somewhere valid), `col_chars` to that line's own length excluding its
    /// terminator (a click past the end of a short line lands at
    /// end-of-line, not out-of-bounds mid-terminator).
    pub fn set_cursor_to_line_col(&mut self, line: usize, col_chars: usize) {
        let doc_len_lines = self.document.len_lines();
        let line = line.min(doc_len_lines.saturating_sub(1));
        let Ok(line_start) = self.document.line_to_char(line) else {
            return;
        };
        let line_len_chars = self
            .document
            .line(line)
            .map(|l| {
                l.strip_suffix("\r\n")
                    .or_else(|| l.strip_suffix('\n'))
                    .unwrap_or(&l)
                    .chars()
                    .count()
            })
            .unwrap_or(0);
        self.cursor = line_start + col_chars.min(line_len_chars);
    }

    /// Moves the cursor one char left, clamped at document start. Returns
    /// whether the cursor actually moved, matching `Viewport::scroll_by`'s
    /// own "did anything change" convention -- callers use this to skip a
    /// redundant redraw when already at a boundary (e.g. repeated
    /// `ArrowLeft` at the very start of the document).
    pub fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Moves the cursor one char right, clamped at document end.
    pub fn move_right(&mut self) -> bool {
        let len = self.document.len_chars();
        if self.cursor >= len {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// Moves the cursor up one line, keeping the current column where
    /// possible (clamped to the shorter line, via `set_cursor_to_line_col`).
    /// A no-op already at line 0. Deliberately does not remember a "desired
    /// column" across a run of up/down moves through lines of different
    /// lengths (the way most real editors do) -- a real, named, minor UX
    /// gap, not a correctness bug: each individual move still lands
    /// somewhere valid, it just re-derives the column from the *current*
    /// (possibly already-clamped) position rather than an original one.
    pub fn move_up(&mut self) -> bool {
        let (line, col) = self.cursor_line_col();
        if line == 0 {
            return false;
        }
        self.set_cursor_to_line_col(line - 1, col);
        true
    }

    /// Moves the cursor down one line, same column-preservation caveat as
    /// `move_up`. A no-op already on the document's last line.
    pub fn move_down(&mut self) -> bool {
        let (line, col) = self.cursor_line_col();
        if line + 1 >= self.document.len_lines() {
            return false;
        }
        self.set_cursor_to_line_col(line + 1, col);
        true
    }
}
