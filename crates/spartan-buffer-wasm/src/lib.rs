//! Real, production wasm-bindgen wrapper around `spartan_buffer::Document`
//! -- the exact same rope/branching-undo-tree engine the desktop shells
//! already depend on, no fork. Promoted from `spikes/wasm-buffer-spike`
//! (§75.85, the real Tier 0 feasibility spike that first confirmed this
//! crate compiles to `wasm32-unknown-unknown` and runs correctly in a
//! real JS engine) once it had a real consumer: `web/`'s client-side
//! editing core (§75.89, task #81), the first real increment of the
//! hybrid web app's pure client-side half.
//!
//! A fuller real API than the spike's own minimal proof-of-concept
//! (`new`/`text`/`insert`/`delete`/`undo`/`len_chars`) -- this crate also
//! exposes `replace`/`text_between`/`len_lines`/`char_to_line`/
//! `line_to_char`/`line`, the real set a real editor UI needs. **Redo** is
//! now real too: `Document` itself has no built-in redo (its own branching
//! undo tree has no single well-defined "redo" -- see `spartan-buffer`'s
//! own doc comment), so this crate builds it as a thin `redo_stack` one
//! layer above `Document` -- the exact same real pattern
//! `spartan-editor-core::EditorView` (§75.19) and
//! `spartan-backend::BackendState` (§75.62) already established, ported
//! here verbatim: `undo` pushes the pre-undo checkpoint, `redo` pops and
//! jumps forward to it, and any real edit clears the stack.

use spartan_buffer::{CheckpointId, Document};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmDocument {
    inner: Document,
    /// Real redo stack -- pre-undo checkpoints `undo` pushes, popped by
    /// `redo`. Matches `spartan-backend::BackendState::redo_stack` exactly.
    redo_stack: Vec<CheckpointId>,
}

#[wasm_bindgen]
impl WasmDocument {
    #[wasm_bindgen(constructor)]
    pub fn new(initial_text: &str) -> WasmDocument {
        WasmDocument {
            inner: Document::new(initial_text),
            redo_stack: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        self.inner.text()
    }

    pub fn len_chars(&self) -> usize {
        self.inner.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.inner.len_lines()
    }

    pub fn insert(&mut self, char_idx: usize, text: &str) -> Result<(), String> {
        let r = self.inner.insert(char_idx, text).map_err(|e| e.to_string());
        if r.is_ok() {
            self.redo_stack.clear();
        }
        r
    }

    pub fn delete(&mut self, start: usize, end: usize) -> Result<(), String> {
        let r = self.inner.delete(start..end).map_err(|e| e.to_string());
        if r.is_ok() {
            self.redo_stack.clear();
        }
        r
    }

    /// Real, single-checkpoint replace of a char range with new text --
    /// the same one real primitive `spartan-backend::edit` (§75.59) uses
    /// for insert/delete/selection-replace alike, matching that same
    /// established pattern here rather than special-casing each shape.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) -> Result<(), String> {
        let r = self
            .inner
            .replace(start..end, text)
            .map_err(|e| e.to_string());
        if r.is_ok() {
            self.redo_stack.clear();
        }
        r
    }

    pub fn text_between(&self, start: usize, end: usize) -> Result<String, String> {
        self.inner
            .text_between(start..end)
            .map_err(|e| e.to_string())
    }

    pub fn undo(&mut self) -> bool {
        let pre_undo = self.inner.current_checkpoint();
        let changed = self.inner.undo();
        if changed {
            self.redo_stack.push(pre_undo);
        }
        changed
    }

    /// Real redo -- pops the pre-undo checkpoint `undo` pushed and jumps
    /// forward to it (`Document` has no single well-defined redo on its own
    /// branching tree). Returns `false` when there's nothing to redo, or if
    /// the checkpoint aged out of `Document`'s bounded ring since the undo
    /// -- the same graceful "skip an evicted checkpoint" fallback
    /// `spartan-backend::redo` (§75.62) already uses.
    pub fn redo(&mut self) -> bool {
        match self.redo_stack.pop() {
            Some(checkpoint) => self.inner.jump_to_checkpoint(checkpoint).is_ok(),
            None => false,
        }
    }

    pub fn char_to_line(&self, char_idx: usize) -> Result<usize, String> {
        self.inner.char_to_line(char_idx).map_err(|e| e.to_string())
    }

    pub fn line_to_char(&self, line_idx: usize) -> Result<usize, String> {
        self.inner.line_to_char(line_idx).map_err(|e| e.to_string())
    }

    pub fn line(&self, line_idx: usize) -> Result<String, String> {
        self.inner.line(line_idx).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_delete_and_replace_all_round_trip_through_the_wrapper() {
        let mut doc = WasmDocument::new("hello world");
        doc.insert(5, ",").unwrap();
        assert_eq!(doc.text(), "hello, world");
        doc.delete(0, 5).unwrap();
        assert_eq!(doc.text(), ", world");
        doc.replace(0, 1, "").unwrap();
        assert_eq!(doc.text(), " world");
    }

    #[test]
    fn undo_restores_the_prior_real_checkpoint() {
        let mut doc = WasmDocument::new("hello world");
        doc.insert(5, ",").unwrap();
        assert!(doc.undo());
        assert_eq!(doc.text(), "hello world");
    }

    #[test]
    fn redo_restores_a_real_undone_edit() {
        let mut doc = WasmDocument::new("hello world");
        doc.insert(5, ",").unwrap();
        assert!(doc.undo());
        assert_eq!(doc.text(), "hello world");
        assert!(doc.redo());
        assert_eq!(doc.text(), "hello, world");
    }

    #[test]
    fn redo_with_nothing_to_redo_is_false() {
        let mut doc = WasmDocument::new("hi");
        assert!(!doc.redo());
        // A real edit with no prior undo still has nothing to redo.
        doc.insert(2, "!").unwrap();
        assert!(!doc.redo());
    }

    #[test]
    fn a_real_new_edit_after_undo_clears_the_redo_stack() {
        let mut doc = WasmDocument::new("hello");
        doc.insert(5, " world").unwrap();
        assert!(doc.undo());
        assert_eq!(doc.text(), "hello");
        // A real new edit must invalidate the redo stack (matching every
        // other real UI surface in this project).
        doc.insert(5, "!").unwrap();
        assert!(!doc.redo());
        assert_eq!(doc.text(), "hello!");
    }

    #[test]
    fn line_and_char_index_conversions_are_real_and_consistent() {
        let doc = WasmDocument::new("one\ntwo\nthree");
        assert_eq!(doc.len_lines(), 3);
        assert_eq!(doc.line(1).unwrap(), "two\n");
        let start_of_line_1 = doc.line_to_char(1).unwrap();
        assert_eq!(doc.char_to_line(start_of_line_1).unwrap(), 1);
    }

    #[test]
    fn text_between_returns_a_real_substring() {
        let doc = WasmDocument::new("hello world");
        assert_eq!(doc.text_between(0, 5).unwrap(), "hello");
        assert_eq!(doc.text_between(6, 11).unwrap(), "world");
    }

    #[test]
    fn an_out_of_range_edit_is_a_real_string_error_not_a_panic() {
        let mut doc = WasmDocument::new("hi");
        assert!(doc.insert(999, "x").is_err());
        assert!(doc.text_between(0, 999).is_err());
    }
}
