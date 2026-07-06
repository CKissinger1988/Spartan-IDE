//! Real tree-sitter syntax highlighting -- Rust only, for now (§75.11).
//! Every `LanguageProfile` has carried an unused `tree_sitter_grammar`
//! field since §75.5; this is the first real wiring of it. Deliberately
//! windowed, not whole-document (see the crate README/§75.11 for why):
//! `tree_sitter_highlight::Highlighter::highlight()`'s public API always
//! scans its entire input (confirmed by reading the real installed
//! `tree-sitter-highlight-0.25.10` source: the byte range is hardcoded to
//! `0..usize::MAX`), so this crate only ever hands it the same ~40-60 line
//! windowed slice everything else in this crate is already scoped to --
//! never the whole document. A real, named consequence: a multi-line
//! construct (a block comment, a raw string) that starts above the visible
//! window will be misinterpreted within it, since the parser has no
//! context above the window's first line.

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

/// A deliberately small, fixed "theme" for this first pass -- not the full
/// breadth of capture names `tree-sitter-rust`'s bundled query defines.
/// Names not in this list are left with no color (cosmic-text's own
/// default), matching `tree_sitter_highlight`'s documented behavior for
/// capture names a `HighlightConfiguration` wasn't `configure()`d with.
///
/// `"constant"`, not `"number"`, is deliberate: a real visual-verification
/// screenshot (§75.11) showed integer/float literals rendering in the
/// default color, uncolored. Reading tree-sitter-rust's actual bundled
/// `queries/highlights.scm` (not assumed from the capture name's obvious
/// English meaning) showed why -- it captures numeric literals as
/// `@constant.builtin`, never `@number`. `tree_sitter_highlight::
/// HighlightConfiguration::configure()`'s real matching rule (read from its
/// installed source) is: a configured name matches a query capture if every
/// dot-separated part of the configured name is present among the parts of
/// the capture name -- so `"constant"` (one part) matches both plain
/// `@constant` (SCREAMING_CASE identifiers) and `@constant.builtin`
/// (numeric/bool literals), the same way this module's existing
/// `"function"` entry already matched `@function.macro` for `println!`.
const HIGHLIGHT_NAMES: &[&str] = &[
    "keyword", "string", "comment", "function", "type", "constant",
];

fn color_for(name: &str) -> glyphon::Color {
    match name {
        "keyword" => glyphon::Color::rgb(0xC7, 0x92, 0xEA),
        "string" => glyphon::Color::rgb(0x9E, 0xCE, 0x6A),
        "comment" => glyphon::Color::rgb(0x62, 0x72, 0xA4),
        "function" => glyphon::Color::rgb(0x7A, 0xA2, 0xF7),
        "type" => glyphon::Color::rgb(0xE0, 0xAF, 0x68),
        "constant" => glyphon::Color::rgb(0xFF, 0x9E, 0x64),
        // Matches `TextState::prepare`'s existing `default_color`.
        _ => glyphon::Color::rgb(0xE9, 0xE7, 0xE4),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub color: glyphon::Color,
}

/// Owns a real tree-sitter parser configuration for one language. Only
/// Rust is wired up in this pass -- matching every prior pass's own
/// precedent (LSP started with `rust-analyzer`, DAP with `lldb-dap`).
pub struct Highlighter {
    inner: TsHighlighter,
    config: HighlightConfiguration,
}

impl Highlighter {
    /// Builds the one real, verified `HighlightConfiguration` for
    /// `tree-sitter-rust`. `tree_sitter_rust::HIGHLIGHTS_QUERY` is a real
    /// bundled `&str` constant the grammar crate ships -- no `.scm` file
    /// needed for this pass.
    pub fn rust() -> Self {
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let mut config = HighlightConfiguration::new(
            language,
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-rust's own bundled highlights query must be valid");
        config.configure(HIGHLIGHT_NAMES);
        Self {
            inner: TsHighlighter::new(),
            config,
        }
    }

    /// Runs a real tree-sitter parse + highlight query pass over `source`
    /// (the currently *windowed* text, not the whole document -- see this
    /// module's doc comment) and returns the resulting colored spans.
    /// Highlight events can nest (e.g. a macro call inside a larger
    /// expression); a stack tracks the currently-active colors so a
    /// `Source` event always uses the innermost (most specific) one.
    pub fn highlight(&mut self, source: &str) -> Vec<HighlightSpan> {
        let bytes = source.as_bytes();
        let events = match self.inner.highlight(&self.config, bytes, None, |_| None) {
            Ok(events) => events,
            Err(_) => return Vec::new(),
        };

        let mut spans = Vec::new();
        let mut color_stack: Vec<glyphon::Color> = Vec::new();
        for event in events {
            let Ok(event) = event else { continue };
            match event {
                HighlightEvent::Source { start, end } => {
                    if let Some(&color) = color_stack.last() {
                        spans.push(HighlightSpan {
                            start_byte: start,
                            end_byte: end,
                            color,
                        });
                    }
                }
                HighlightEvent::HighlightStart(h) => {
                    color_stack.push(color_for(HIGHLIGHT_NAMES[h.0]));
                }
                HighlightEvent::HighlightEnd => {
                    color_stack.pop();
                }
            }
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_keyword_gets_a_real_span_at_the_right_byte_range() {
        let mut hl = Highlighter::rust();
        let spans = hl.highlight("fn main() {}");
        let fn_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == 2)
            .expect("expected a highlight span covering 'fn'");
        assert_eq!(fn_span.color, color_for("keyword"));
    }

    #[test]
    fn a_real_string_literal_gets_a_real_span() {
        let mut hl = Highlighter::rust();
        let source = r#"fn main() { let s = "hello"; }"#;
        let spans = hl.highlight(source);
        let string_start = source.find('"').unwrap();
        let string_end = source.rfind('"').unwrap() + 1;
        let string_span = spans
            .iter()
            .find(|s| s.start_byte == string_start && s.end_byte == string_end)
            .unwrap_or_else(|| {
                panic!("expected a string span at {string_start}..{string_end}, got: {spans:?}")
            });
        assert_eq!(string_span.color, color_for("string"));
    }

    #[test]
    fn a_real_line_comment_gets_a_real_span() {
        let mut hl = Highlighter::rust();
        let comment_text = "// a real comment";
        let source = format!("{comment_text}\nfn main() {{}}");
        let spans = hl.highlight(&source);
        // Real behavior, found by running this rather than assumed: the
        // comment span covers the comment text itself, not the trailing
        // newline that ends the line (the newline is a separate token).
        let comment_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == comment_text.len())
            .unwrap_or_else(|| {
                panic!("expected a comment span covering the comment text, got: {spans:?}")
            });
        assert_eq!(comment_span.color, color_for("comment"));
    }

    #[test]
    fn a_real_integer_literal_gets_a_real_span() {
        // Locks in a real bug this pass's own visual verification caught on
        // screen (numbers rendering uncolored) and traced to tree-sitter-
        // rust's bundled query using `@constant.builtin`, never `@number`,
        // for literals -- see this module's `HIGHLIGHT_NAMES` doc comment.
        let mut hl = Highlighter::rust();
        let source = "fn main() { let x = 42; }";
        let spans = hl.highlight(source);
        let number_start = source.find("42").unwrap();
        let number_span = spans
            .iter()
            .find(|s| s.start_byte == number_start && s.end_byte == number_start + 2)
            .unwrap_or_else(|| panic!("expected a span covering the literal '42', got: {spans:?}"));
        assert_eq!(number_span.color, color_for("constant"));
    }
}
