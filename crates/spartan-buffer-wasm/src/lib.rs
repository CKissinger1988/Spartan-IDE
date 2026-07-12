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
//! `line_to_char`/`line`, the real set a real editor UI needs. Real,
//! deliberate scope cut, named honestly: no `redo` yet. `Document` itself
//! has no built-in redo (its own branching undo tree has no single
//! well-defined "redo" -- see `spartan-buffer`'s own doc comment); every
//! other real UI surface in this project builds redo as a thin
//! `redo_stack` one layer above `Document` (`spartan-editor-core::
//! EditorView`, §75.19; `spartan-backend::BackendState`, §75.62) rather
//! than inside it -- the same real, separate follow-up piece here, not
//! attempted in this pass.

use spartan_buffer::Document;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmDocument {
    inner: Document,
}

#[wasm_bindgen]
impl WasmDocument {
    #[wasm_bindgen(constructor)]
    pub fn new(initial_text: &str) -> WasmDocument {
        WasmDocument {
            inner: Document::new(initial_text),
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
        self.inner.insert(char_idx, text).map_err(|e| e.to_string())
    }

    pub fn delete(&mut self, start: usize, end: usize) -> Result<(), String> {
        self.inner.delete(start..end).map_err(|e| e.to_string())
    }

    /// Real, single-checkpoint replace of a char range with new text --
    /// the same one real primitive `spartan-backend::edit` (§75.59) uses
    /// for insert/delete/selection-replace alike, matching that same
    /// established pattern here rather than special-casing each shape.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) -> Result<(), String> {
        self.inner
            .replace(start..end, text)
            .map_err(|e| e.to_string())
    }

    pub fn text_between(&self, start: usize, end: usize) -> Result<String, String> {
        self.inner
            .text_between(start..end)
            .map_err(|e| e.to_string())
    }

    pub fn undo(&mut self) -> bool {
        self.inner.undo()
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
