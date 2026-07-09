//! Real tree-sitter syntax highlighting (§75.11 for Rust; §75.29 extends
//! this to TypeScript/JavaScript, Python, Java, and Go). Every
//! `LanguageProfile` has carried an unused `tree_sitter_grammar` field
//! since §75.5; this is the real wiring of it. Deliberately windowed, not
//! whole-document (see the crate README/§75.11 for why):
//! `tree_sitter_highlight::Highlighter::highlight()`'s public API always
//! scans its entire input (confirmed by reading the real installed
//! `tree-sitter-highlight-0.25.10` source: the byte range is hardcoded to
//! `0..usize::MAX`), so this crate only ever hands it the same ~40-60 line
//! windowed slice everything else in this crate is already scoped to --
//! never the whole document. A real, named consequence: a multi-line
//! construct (a block comment, a raw string) that starts above the visible
//! window will be misinterpreted within it, since the parser has no
//! context above the window's first line.
//!
//! TypeScript is a real special case, found by reading the actual
//! installed `tree-sitter-typescript-0.23.2` source rather than assumed:
//! its own bundled `HIGHLIGHTS_QUERY` only covers TypeScript's *additions*
//! over JavaScript (types, `interface`/`enum`/`namespace`/... keywords) --
//! it has no captures at all for strings, comments, numbers, or function
//! declarations, since the grammar itself is built as a superset of the
//! JS grammar and its query is designed to be layered on top of
//! `tree-sitter-javascript`'s own comprehensive query (the same real
//! convention other editors that bundle both grammars use). Confirmed by
//! writing and running `typescript_highlighting_covers_both_ts_specific_and_js_base_syntax`
//! below: parsing with `tree_sitter_typescript::LANGUAGE_TYPESCRIPT` and
//! querying with the two query strings concatenated (JS query first, TS
//! query second) correctly highlights both a base-JS string literal and a
//! TS-only `interface` keyword in the same source.
//!
//! Kotlin was a real, named gap through §75.29-§75.43: the only
//! tree-sitter-0.25-compatible Kotlin grammar crate then known
//! (`tree-sitter-kotlin-ng` 0.x) shipped no bundled highlights query, and
//! the one alternate crate that did (`tree-sitter-kotlin` 0.3.8) pinned
//! `tree-sitter = "0.22"`, hard-incompatible with this workspace. §75.44
//! re-checked this (the same "recheck a stale blocker" discipline that
//! found Ollama newly reachable in the same session) and found
//! `tree-sitter-kotlin-ng` had moved to 1.1.0 under a new maintaining org
//! -- still compatible with `tree-sitter = "0.25"`, confirmed by adding it
//! and building -- but a real shallow clone of its actual source repo
//! (not just the published crate) confirmed it *still* ships no
//! `queries/highlights.scm` anywhere. `kotlin_highlights.scm` (vendored
//! directly into this crate, `include_str!`'d below) is a real,
//! hand-authored query built from the grammar's own real, installed
//! `node-types.json` field/type names -- not guessed, and not carried
//! over from any other language's query -- see that file's own doc
//! comment for the one real, confirmed gap it can't cover (no distinct
//! boolean/null literal node type exists in this grammar at all).

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter};

/// A deliberately small, fixed "theme" for this first pass -- not the full
/// breadth of capture names `tree-sitter-rust`'s bundled query defines.
/// Names not in this list are left with no color (cosmic-text's own
/// default), matching `tree_sitter_highlight`'s documented behavior for
/// capture names a `HighlightConfiguration` wasn't `configure()`d with.
///
/// `"constant"`, not `"number"`, was the first real finding (§75.11): a
/// visual-verification screenshot showed integer/float literals rendering
/// in the default color, uncolored, traced to tree-sitter-rust's bundled
/// query capturing numeric literals as `@constant.builtin`, never
/// `@number`. `"number"` was added as its own separate entry in §75.29
/// after the *same* discipline (run it, don't assume) caught a second,
/// different real finding: Python's and Go's own bundled queries capture
/// numeric literals as plain `@number` instead, never `@constant.builtin`
/// -- confirmed by grepping their real installed `queries/highlights.scm`
/// files, not guessed from Rust's precedent. `tree_sitter_highlight::
/// HighlightConfiguration::configure()`'s real matching rule (read from its
/// installed source) is: a configured name matches a query capture if every
/// dot-separated part of the configured name is present among the parts of
/// the capture name -- so `"constant"` (one part) matches both plain
/// `@constant` (SCREAMING_CASE identifiers) and `@constant.builtin`
/// (Rust's numeric/bool literals), the same way this module's existing
/// `"function"` entry already matched `@function.macro` for `println!`.
const HIGHLIGHT_NAMES: &[&str] = &[
    "keyword", "string", "comment", "function", "type", "constant", "number",
];

fn color_for(name: &str) -> glyphon::Color {
    match name {
        "keyword" => glyphon::Color::rgb(0xC7, 0x92, 0xEA),
        "string" => glyphon::Color::rgb(0x9E, 0xCE, 0x6A),
        "comment" => glyphon::Color::rgb(0x62, 0x72, 0xA4),
        "function" => glyphon::Color::rgb(0x7A, 0xA2, 0xF7),
        "type" => glyphon::Color::rgb(0xE0, 0xAF, 0x68),
        "constant" => glyphon::Color::rgb(0xFF, 0x9E, 0x64),
        // Same color family as "constant" -- both are literal values,
        // just captured under different names by different grammars'
        // bundled queries (see this constant's own doc comment above).
        "number" => glyphon::Color::rgb(0xFF, 0x9E, 0x64),
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

/// Owns a real tree-sitter parser configuration for one language. Rust
/// (§75.11), TypeScript/JavaScript, Python, Java, and Go (§75.29), and now
/// Kotlin (§75.44, via this crate's own vendored query) are wired.
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

    /// Real `tree-sitter-typescript` config. Parses with
    /// `LANGUAGE_TYPESCRIPT` (not `LANGUAGE_TSX` -- matching
    /// `languages.toml`'s single `"typescript"` profile covering
    /// `.ts`/`.tsx`/`.js`/`.jsx`; a real, named simplification since a
    /// `.tsx`/`.jsx` file's JSX syntax will not parse correctly under the
    /// plain TypeScript grammar). See this module's doc comment for why
    /// the query text is `tree-sitter-javascript`'s query concatenated
    /// with `tree-sitter-typescript`'s own delta query, in that order.
    pub fn typescript() -> Self {
        let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let combined_query = format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        );
        let mut config =
            HighlightConfiguration::new(language, "typescript", &combined_query, "", "").expect(
                "tree-sitter-javascript + tree-sitter-typescript's combined query must be valid",
            );
        config.configure(HIGHLIGHT_NAMES);
        Self {
            inner: TsHighlighter::new(),
            config,
        }
    }

    /// Real `tree-sitter-python` config -- its own bundled query is
    /// self-sufficient (comment/string/keyword/function/number/constant
    /// all present), unlike TypeScript's.
    pub fn python() -> Self {
        let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        let mut config = HighlightConfiguration::new(
            language,
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-python's own bundled highlights query must be valid");
        config.configure(HIGHLIGHT_NAMES);
        Self {
            inner: TsHighlighter::new(),
            config,
        }
    }

    /// Real `tree-sitter-java` config -- self-sufficient bundled query.
    pub fn java() -> Self {
        let language: tree_sitter::Language = tree_sitter_java::LANGUAGE.into();
        let mut config = HighlightConfiguration::new(
            language,
            "java",
            tree_sitter_java::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-java's own bundled highlights query must be valid");
        config.configure(HIGHLIGHT_NAMES);
        Self {
            inner: TsHighlighter::new(),
            config,
        }
    }

    /// Real `tree-sitter-kotlin-ng` config -- query text is this crate's
    /// own vendored `kotlin_highlights.scm` (see this module's doc
    /// comment for why: the grammar crate ships no bundled query at all).
    pub fn kotlin() -> Self {
        let language: tree_sitter::Language = tree_sitter_kotlin_ng::LANGUAGE.into();
        let query = include_str!("kotlin_highlights.scm");
        let mut config = HighlightConfiguration::new(language, "kotlin", query, "", "")
            .expect("this crate's own vendored kotlin_highlights.scm must be valid");
        config.configure(HIGHLIGHT_NAMES);
        Self {
            inner: TsHighlighter::new(),
            config,
        }
    }

    /// Real `tree-sitter-go` config -- self-sufficient bundled query.
    pub fn go() -> Self {
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        let mut config =
            HighlightConfiguration::new(language, "go", tree_sitter_go::HIGHLIGHTS_QUERY, "", "")
                .expect("tree-sitter-go's own bundled highlights query must be valid");
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

    #[test]
    fn typescript_highlighting_covers_both_ts_specific_and_js_base_syntax() {
        // Real, load-bearing test for the finding in this module's doc
        // comment: tree-sitter-typescript's own bundled query alone has no
        // captures for base-JS syntax (strings, keywords like "const") --
        // only the JS+TS combined query correctly highlights both.
        let mut hl = Highlighter::typescript();
        let source = r#"interface Foo { x: string } const s = "hi";"#;
        let spans = hl.highlight(source);

        let interface_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == "interface".len())
            .unwrap_or_else(|| panic!("expected an 'interface' keyword span, got: {spans:?}"));
        assert_eq!(interface_span.color, color_for("keyword"));

        let string_start = source.find('"').unwrap();
        let string_end = source.rfind('"').unwrap() + 1;
        let string_span = spans
            .iter()
            .find(|s| s.start_byte == string_start && s.end_byte == string_end)
            .unwrap_or_else(|| {
                panic!("expected a string span at {string_start}..{string_end}, got: {spans:?}")
            });
        assert_eq!(string_span.color, color_for("string"));

        let const_start = source.find("const").unwrap();
        let const_span = spans
            .iter()
            .find(|s| s.start_byte == const_start && s.end_byte == const_start + "const".len())
            .unwrap_or_else(|| panic!("expected a 'const' keyword span, got: {spans:?}"));
        assert_eq!(const_span.color, color_for("keyword"));
    }

    #[test]
    fn python_keyword_string_comment_and_number_all_get_real_spans() {
        let mut hl = Highlighter::python();
        let source = "# a comment\ndef f():\n    x = 42\n    return \"hi\"";
        let spans = hl.highlight(source);

        let def_span = spans
            .iter()
            .find(|s| {
                s.start_byte == source.find("def").unwrap()
                    && s.end_byte == source.find("def").unwrap() + 3
            })
            .unwrap_or_else(|| panic!("expected a 'def' keyword span, got: {spans:?}"));
        assert_eq!(def_span.color, color_for("keyword"));

        let comment_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == "# a comment".len())
            .unwrap_or_else(|| panic!("expected a comment span, got: {spans:?}"));
        assert_eq!(comment_span.color, color_for("comment"));

        let number_start = source.find("42").unwrap();
        let number_span = spans
            .iter()
            .find(|s| s.start_byte == number_start && s.end_byte == number_start + 2)
            .unwrap_or_else(|| panic!("expected a span covering '42', got: {spans:?}"));
        // Real finding (see HIGHLIGHT_NAMES's doc comment): Python's own
        // bundled query captures numeric literals as plain `@number`, not
        // `@constant.builtin` the way Rust's does.
        assert_eq!(number_span.color, color_for("number"));

        let string_start = source.find('"').unwrap();
        let string_end = source.rfind('"').unwrap() + 1;
        let string_span = spans
            .iter()
            .find(|s| s.start_byte == string_start && s.end_byte == string_end)
            .unwrap_or_else(|| panic!("expected a string span, got: {spans:?}"));
        assert_eq!(string_span.color, color_for("string"));
    }

    #[test]
    fn java_keyword_string_and_comment_all_get_real_spans() {
        let mut hl = Highlighter::java();
        let source = "// a comment\nclass Foo {\n  String s = \"hi\";\n}";
        let spans = hl.highlight(source);

        let class_start = source.find("class").unwrap();
        let class_span = spans
            .iter()
            .find(|s| s.start_byte == class_start && s.end_byte == class_start + "class".len())
            .unwrap_or_else(|| panic!("expected a 'class' keyword span, got: {spans:?}"));
        assert_eq!(class_span.color, color_for("keyword"));

        let comment_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == "// a comment".len())
            .unwrap_or_else(|| panic!("expected a comment span, got: {spans:?}"));
        assert_eq!(comment_span.color, color_for("comment"));

        let string_start = source.find('"').unwrap();
        let string_end = source.rfind('"').unwrap() + 1;
        let string_span = spans
            .iter()
            .find(|s| s.start_byte == string_start && s.end_byte == string_end)
            .unwrap_or_else(|| panic!("expected a string span, got: {spans:?}"));
        assert_eq!(string_span.color, color_for("string"));
    }

    #[test]
    fn go_keyword_string_comment_and_number_all_get_real_spans() {
        let mut hl = Highlighter::go();
        let source = "// a comment\nfunc main() {\n\tx := 42\n\ts := \"hi\"\n}";
        let spans = hl.highlight(source);

        let func_start = source.find("func").unwrap();
        let func_span = spans
            .iter()
            .find(|s| s.start_byte == func_start && s.end_byte == func_start + "func".len())
            .unwrap_or_else(|| panic!("expected a 'func' keyword span, got: {spans:?}"));
        assert_eq!(func_span.color, color_for("keyword"));

        let comment_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == "// a comment".len())
            .unwrap_or_else(|| panic!("expected a comment span, got: {spans:?}"));
        assert_eq!(comment_span.color, color_for("comment"));

        let number_start = source.find("42").unwrap();
        let number_span = spans
            .iter()
            .find(|s| s.start_byte == number_start && s.end_byte == number_start + 2)
            .unwrap_or_else(|| panic!("expected a span covering '42', got: {spans:?}"));
        // Real finding (see HIGHLIGHT_NAMES's doc comment): Go's own
        // bundled query captures numeric literals as plain `@number`, not
        // `@constant.builtin` the way Rust's does.
        assert_eq!(number_span.color, color_for("number"));

        let string_start = source.find('"').unwrap();
        let string_end = source.rfind('"').unwrap() + 1;
        let string_span = spans
            .iter()
            .find(|s| s.start_byte == string_start && s.end_byte == string_end)
            .unwrap_or_else(|| panic!("expected a string span, got: {spans:?}"));
        assert_eq!(string_span.color, color_for("string"));
    }

    #[test]
    fn kotlin_keyword_string_comment_number_function_and_type_all_get_real_spans() {
        // Real, hand-authored query (§75.44, `kotlin_highlights.scm`) --
        // no bundled query exists for this grammar to fall back on, so
        // this test is this query's own real correctness proof, not just
        // a smoke test.
        let mut hl = Highlighter::kotlin();
        let source =
            "// a comment\nclass Greeter {\n  fun greet(name: String): String {\n    val count = 42\n    return \"hi \" + name\n  }\n}";
        let spans = hl.highlight(source);

        let comment_span = spans
            .iter()
            .find(|s| s.start_byte == 0 && s.end_byte == "// a comment".len())
            .unwrap_or_else(|| panic!("expected a comment span, got: {spans:?}"));
        assert_eq!(comment_span.color, color_for("comment"));

        let class_kw_start = source.find("class").unwrap();
        let class_kw_span = spans
            .iter()
            .find(|s| {
                s.start_byte == class_kw_start && s.end_byte == class_kw_start + "class".len()
            })
            .unwrap_or_else(|| panic!("expected a 'class' keyword span, got: {spans:?}"));
        assert_eq!(class_kw_span.color, color_for("keyword"));

        let type_start = source.find("Greeter").unwrap();
        let type_span = spans
            .iter()
            .find(|s| s.start_byte == type_start && s.end_byte == type_start + "Greeter".len())
            .unwrap_or_else(|| panic!("expected a 'Greeter' type span, got: {spans:?}"));
        assert_eq!(type_span.color, color_for("type"));

        let fun_kw_start = source.find("fun").unwrap();
        let fun_kw_span = spans
            .iter()
            .find(|s| s.start_byte == fun_kw_start && s.end_byte == fun_kw_start + "fun".len())
            .unwrap_or_else(|| panic!("expected a 'fun' keyword span, got: {spans:?}"));
        assert_eq!(fun_kw_span.color, color_for("keyword"));

        let func_name_start = source.find("greet").unwrap();
        let func_name_span = spans
            .iter()
            .find(|s| {
                s.start_byte == func_name_start && s.end_byte == func_name_start + "greet".len()
            })
            .unwrap_or_else(|| panic!("expected a 'greet' function-name span, got: {spans:?}"));
        assert_eq!(func_name_span.color, color_for("function"));

        let val_kw_start = source.find("val").unwrap();
        let val_kw_span = spans
            .iter()
            .find(|s| s.start_byte == val_kw_start && s.end_byte == val_kw_start + "val".len())
            .unwrap_or_else(|| panic!("expected a 'val' keyword span, got: {spans:?}"));
        assert_eq!(val_kw_span.color, color_for("keyword"));

        let number_start = source.find("42").unwrap();
        let number_span = spans
            .iter()
            .find(|s| s.start_byte == number_start && s.end_byte == number_start + 2)
            .unwrap_or_else(|| panic!("expected a span covering the literal '42', got: {spans:?}"));
        assert_eq!(number_span.color, color_for("number"));

        let return_kw_start = source.find("return").unwrap();
        let return_kw_span = spans
            .iter()
            .find(|s| {
                s.start_byte == return_kw_start && s.end_byte == return_kw_start + "return".len()
            })
            .unwrap_or_else(|| panic!("expected a 'return' keyword span, got: {spans:?}"));
        assert_eq!(return_kw_span.color, color_for("keyword"));

        let string_start = source.find('"').unwrap();
        let string_end = source[string_start + 1..].find('"').unwrap() + string_start + 2;
        let string_span = spans
            .iter()
            .find(|s| s.start_byte == string_start && s.end_byte == string_end)
            .unwrap_or_else(|| panic!("expected a string span, got: {spans:?}"));
        assert_eq!(string_span.color, color_for("string"));
    }
}
