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
    /// Real undo/redo (§75.19): checkpoint IDs undone *away from*, most
    /// recent last -- pushed by `undo()` on success, popped and jumped back
    /// to by `redo()`, cleared by any new real edit. Lives here, not in
    /// `Document`, because `spartan-buffer`'s undo tree only models "move
    /// to parent" / "jump anywhere"; it deliberately has no linear redo
    /// concept of its own (§2.1's branching design means "redo" isn't
    /// always well-defined -- jumping to an arbitrary sibling checkpoint is
    /// just as valid a "redo" as this one), so the conventional
    /// most-recently-undone-wins behavior is built here, one layer up.
    redo_stack: Vec<spartan_buffer::CheckpointId>,
    /// "Sticky column" (§75.22): the column an up/down *run* tries to return
    /// to, even after passing through a shorter intermediate line that would
    /// otherwise clamp it away permanently -- matching conventional editor
    /// behavior (e.g. arrowing down through a blank line and back into a
    /// long one restores the original column, not column 0). Stores the
    /// *desired* column, not whatever `set_cursor_to_line_col` actually
    /// clamped it to on a short line, which is what makes it survive
    /// multiple short lines in a row. `None` when no up/down run is active;
    /// captured by `move_up`/`move_down` on the first move of a run and
    /// reused (not re-derived from the cursor) on every subsequent call in
    /// the same run. Cleared by every other cursor-moving method, since a
    /// run is defined as *consecutive* up/down moves only -- anything else
    /// (left/right, word/line/document jumps, mouse clicks, edits,
    /// undo/redo) intentionally starts a fresh run next time.
    sticky_column: Option<usize>,
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
            redo_stack: Vec::new(),
            sticky_column: None,
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

    /// The active selection's real text content, if any -- real clipboard
    /// copy support (§75.20) needed this and no substring accessor existed
    /// on `Document` before that pass added `text_between`.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        self.document.text_between(start..end).ok()
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
        self.redo_stack.clear();
        self.sticky_column = None;
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
        self.redo_stack.clear();
        self.sticky_column = None;
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
        self.redo_stack.clear();
        self.sticky_column = None;
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
        self.sticky_column = None;
        let doc_len_lines = self.document.len_lines();
        let line = line.min(doc_len_lines.saturating_sub(1));
        let Ok(line_start) = self.document.line_to_char(line) else {
            return;
        };
        let line_len_chars = self.line_len_chars(line);
        self.cursor = line_start + col_chars.min(line_len_chars);
    }

    /// A line's real length in chars, excluding its `\n`/`\r\n` terminator --
    /// factored out of `set_cursor_to_line_col` in §75.22, when
    /// `move_to_line_end` needed the exact same calculation.
    fn line_len_chars(&self, line: usize) -> usize {
        self.document
            .line(line)
            .map(|l| {
                l.strip_suffix("\r\n")
                    .or_else(|| l.strip_suffix('\n'))
                    .unwrap_or(&l)
                    .chars()
                    .count()
            })
            .unwrap_or(0)
    }

    /// Real single-char fetch via `Document::text_between` (§75.22), used
    /// only by the word-jump methods below -- bounded to a handful of calls
    /// per jump (one word's worth of chars, not the whole document), so the
    /// O(log n)-per-call rope slice cost this incurs stays cheap even on a
    /// large file. `spartan-buffer::Document` has no cheaper single-char
    /// accessor of its own.
    fn char_before(&self, pos: usize) -> Option<char> {
        if pos == 0 {
            return None;
        }
        self.document
            .text_between(pos - 1..pos)
            .ok()?
            .chars()
            .next()
    }

    /// The char immediately at (not before) `pos`, mirroring `char_before`.
    fn char_at(&self, pos: usize) -> Option<char> {
        if pos >= self.document.len_chars() {
            return None;
        }
        self.document
            .text_between(pos..pos + 1)
            .ok()?
            .chars()
            .next()
    }

    /// Moves the cursor one char left, clamped at document start. Returns
    /// whether the cursor actually moved, matching `Viewport::scroll_by`'s
    /// own "did anything change" convention -- callers use this to skip a
    /// redundant redraw when already at a boundary (e.g. repeated
    /// `ArrowLeft` at the very start of the document).
    pub fn move_left(&mut self) -> bool {
        self.sticky_column = None;
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    /// Moves the cursor one char right, clamped at document end.
    pub fn move_right(&mut self) -> bool {
        self.sticky_column = None;
        let len = self.document.len_chars();
        if self.cursor >= len {
            return false;
        }
        self.cursor += 1;
        true
    }

    /// Moves the cursor up one line, keeping a "sticky" column where
    /// possible across a whole run of consecutive up/down moves (§75.22) --
    /// see the `sticky_column` field's own doc comment for why this survives
    /// passing through shorter intermediate lines, unlike naively
    /// re-deriving the column from the cursor on every call. A no-op already
    /// at line 0.
    pub fn move_up(&mut self) -> bool {
        let (line, col) = self.cursor_line_col();
        if line == 0 {
            return false;
        }
        let target_col = self.sticky_column.unwrap_or(col);
        self.set_cursor_to_line_col(line - 1, target_col);
        self.sticky_column = Some(target_col);
        true
    }

    /// Moves the cursor down one line, same sticky-column behavior as
    /// `move_up`. A no-op already on the document's last line.
    pub fn move_down(&mut self) -> bool {
        let (line, col) = self.cursor_line_col();
        if line + 1 >= self.document.len_lines() {
            return false;
        }
        let target_col = self.sticky_column.unwrap_or(col);
        self.set_cursor_to_line_col(line + 1, target_col);
        self.sticky_column = Some(target_col);
        true
    }

    /// Moves to the start of the current line (Home). A no-op if the cursor
    /// is already there.
    pub fn move_to_line_start(&mut self) -> bool {
        self.sticky_column = None;
        let (line, col) = self.cursor_line_col();
        if col == 0 {
            return false;
        }
        let Ok(line_start) = self.document.line_to_char(line) else {
            return false;
        };
        self.cursor = line_start;
        true
    }

    /// Moves to the end of the current line, excluding its terminator (End).
    /// A no-op if the cursor is already there.
    pub fn move_to_line_end(&mut self) -> bool {
        self.sticky_column = None;
        let (line, col) = self.cursor_line_col();
        let line_len_chars = self.line_len_chars(line);
        if col >= line_len_chars {
            return false;
        }
        let Ok(line_start) = self.document.line_to_char(line) else {
            return false;
        };
        self.cursor = line_start + line_len_chars;
        true
    }

    /// Moves to the very start of the document (Ctrl+Home). A no-op if the
    /// cursor is already there.
    pub fn move_to_document_start(&mut self) -> bool {
        self.sticky_column = None;
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    /// Moves to the very end of the document (Ctrl+End). A no-op if the
    /// cursor is already there.
    pub fn move_to_document_end(&mut self) -> bool {
        self.sticky_column = None;
        let len = self.document.len_chars();
        if self.cursor >= len {
            return false;
        }
        self.cursor = len;
        true
    }

    /// Moves left to the start of the previous word (Ctrl+Left), matching
    /// conventional editor behavior: any whitespace/newlines immediately
    /// before the cursor are skipped first, then the contiguous run of
    /// "same kind" chars before that -- word chars (alphanumeric/`_`) and
    /// punctuation are treated as different kinds, so `foo.bar| baz` stops
    /// at `foo.|bar` before reaching `foo.| bar` then `|foo.bar baz`, not
    /// just at whitespace. Crosses line boundaries freely since `\n` is
    /// whitespace. A no-op at document start.
    pub fn move_word_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.sticky_column = None;
        let mut pos = self.cursor;
        while pos > 0 && self.char_before(pos).is_some_and(|c| c.is_whitespace()) {
            pos -= 1;
        }
        if pos > 0 {
            let is_word = self.char_before(pos).is_some_and(is_word_char);
            while pos > 0 {
                match self.char_before(pos) {
                    Some(c) if !c.is_whitespace() && is_word_char(c) == is_word => pos -= 1,
                    _ => break,
                }
            }
        }
        self.cursor = pos;
        true
    }

    /// Moves right to the start of the next word (Ctrl+Right), mirroring
    /// `move_word_left`. A no-op at document end.
    pub fn move_word_right(&mut self) -> bool {
        let len = self.document.len_chars();
        if self.cursor >= len {
            return false;
        }
        self.sticky_column = None;
        let mut pos = self.cursor;
        while pos < len && self.char_at(pos).is_some_and(|c| c.is_whitespace()) {
            pos += 1;
        }
        if pos < len {
            let is_word = self.char_at(pos).is_some_and(is_word_char);
            while pos < len {
                match self.char_at(pos) {
                    Some(c) if !c.is_whitespace() && is_word_char(c) == is_word => pos += 1,
                    _ => break,
                }
            }
        }
        self.cursor = pos;
        true
    }

    /// Real undo (§75.19): moves to the parent checkpoint in
    /// `spartan-buffer`'s own branching undo tree, remembering where we
    /// came from (on `self.redo_stack`) so `redo()` can return to it.
    /// Returns whether anything actually changed -- `false` at the root of
    /// the undo tree, or once that history has aged out of the bounded
    /// ring (`Document::undo`'s own two documented cases), matching every
    /// other movement method's "did anything change" convention. Clears
    /// any active selection (an undone edit invalidates whatever the
    /// selection was pointing at) and clamps the cursor into the restored
    /// document's real bounds -- `self.cursor` has no meaning in the
    /// restored rope otherwise.
    pub fn undo(&mut self) -> bool {
        let before = self.document.current_checkpoint();
        if !self.document.undo() {
            return false;
        }
        self.redo_stack.push(before);
        self.selection_anchor = None;
        self.sticky_column = None;
        self.cursor = self.cursor.min(self.document.len_chars());
        true
    }

    /// Real redo: jumps back to the most recently undone-away-from
    /// checkpoint, if any. A checkpoint can legitimately have aged out of
    /// `spartan-buffer`'s bounded ring since it was pushed here (a real,
    /// honest possibility this crate's own design allows for, not a
    /// hypothetical) -- `jump_to_checkpoint` reports that as an `Err`,
    /// which this method treats as "skip it and try the next one" rather
    /// than surfacing an error there's nothing a caller could usefully act
    /// on. Returns whether anything actually changed.
    pub fn redo(&mut self) -> bool {
        while let Some(id) = self.redo_stack.pop() {
            if self.document.jump_to_checkpoint(id).is_ok() {
                self.selection_anchor = None;
                self.sticky_column = None;
                self.cursor = self.cursor.min(self.document.len_chars());
                return true;
            }
        }
        false
    }
}

/// Classifies a char for word-boundary purposes (§75.22): alphanumeric or
/// `_` counts as "word", everything else (punctuation) doesn't -- matching
/// the common "words are identifiers" editor convention rather than
/// natural-language word boundaries. Whitespace is handled separately by
/// callers before this is consulted.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
