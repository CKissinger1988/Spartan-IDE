; Real, deliberately minimal query -- NOT the full production
; tree-sitter-rust highlights.scm. That query (bundled with the current
; crates.io tree-sitter-rust 0.24.2, the same one crates/spartan-editor-core
; uses on the Rust side) references node types (e.g. `doc_comment`) that
; don't exist in the older grammar version tree-sitter-wasms bundled here --
; a real, confirmed version mismatch, see README.md. This subset only uses
; node types confirmed present in both grammar generations.
(line_comment) @comment
(string_literal) @string
(function_item name: (identifier) @function)
(integer_literal) @number
