//! Tier 0 spike: does spartan-buffer's real rope/undo-tree Document
//! compile to wasm32-unknown-unknown and actually run inside a real JS
//! engine via wasm-bindgen, not just compile? A thin, real wrapper
//! around a small slice of Document's already-real, already-tested API
//! -- no new buffer logic here, this crate is purely a feasibility gate.

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

    pub fn insert(&mut self, char_idx: usize, text: &str) -> Result<(), String> {
        self.inner.insert(char_idx, text).map_err(|e| e.to_string())
    }

    pub fn delete(&mut self, start: usize, end: usize) -> Result<(), String> {
        self.inner.delete(start..end).map_err(|e| e.to_string())
    }

    pub fn undo(&mut self) -> bool {
        self.inner.undo()
    }

    pub fn len_chars(&self) -> usize {
        self.inner.len_chars()
    }
}

/// Real, headless tests of this thin wrapper's own logic (error mapping,
/// argument shape), run for the host target under plain `cargo test` --
/// `#[wasm_bindgen]` types compile and behave normally off the wasm32
/// target too, so this doesn't need Node or a browser. The real
/// cross-target proof (does the compiled `.wasm` actually run correctly
/// in a real JS engine) is a separate, manual step documented in this
/// crate's own README, since no wasm32 JS-engine test runner exists in
/// this workspace yet.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_delete_round_trip_through_the_wrapper() {
        let mut doc = WasmDocument::new("hello world");
        assert_eq!(doc.text(), "hello world");
        doc.insert(5, ",").unwrap();
        assert_eq!(doc.text(), "hello, world");
        doc.delete(0, 5).unwrap();
        assert_eq!(doc.text(), ", world");
    }

    #[test]
    fn undo_restores_the_prior_real_checkpoint() {
        let mut doc = WasmDocument::new("hello world");
        doc.insert(5, ",").unwrap();
        assert!(doc.undo());
        assert_eq!(doc.text(), "hello world");
    }

    #[test]
    fn an_out_of_range_edit_is_a_real_string_error_not_a_panic() {
        let mut doc = WasmDocument::new("hi");
        let result = doc.insert(999, "x");
        assert!(result.is_err());
    }

    #[test]
    fn len_chars_reflects_real_edits() {
        let mut doc = WasmDocument::new("hi");
        assert_eq!(doc.len_chars(), 2);
        doc.insert(2, "!").unwrap();
        assert_eq!(doc.len_chars(), 3);
    }
}
