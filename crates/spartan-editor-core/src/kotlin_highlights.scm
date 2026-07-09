; Real, hand-authored highlights query for `tree-sitter-kotlin-ng` 1.1.0
; (§75.44) -- the real, confirmed-live tree-sitter-0.25-compatible Kotlin
; grammar this workspace uses. Written because the crate itself ships none
; (confirmed by a full file listing of both the published crate and a real
; shallow clone of its source repository -- see `highlight.rs`'s own doc
; comment for the full investigation). Node type names and field names below
; come from the grammar's real, installed `src/node-types.json`, not
; guessed; every capture in this file was confirmed against real parsed
; Kotlin fixtures in `highlight.rs`'s own test suite before being kept.
;
; A real, named gap: this grammar has no distinct `true`/`false`/`null`
; literal node type at all (confirmed by listing every real subtype of
; `primary_expression` in `node-types.json` -- unlike Rust's grammar, which
; does distinguish boolean literals syntactically, Kotlin's boolean/null
; literals parse as plain `identifier` nodes here, indistinguishable from
; any other identifier at the syntax level this query operates on) -- so
; they are not, and cannot be, specially highlighted by this query alone.
;
; The keyword list below was NOT hand-extracted from `grammar.js` source
; text (an earlier draft was, and was wrong): a real, throwaway diagnostic
; program iterating `tree_sitter::Language::node_kind_for_id` /
; `node_kind_is_named` / `node_kind_is_visible` over every real symbol id
; in the compiled `tree_sitter_kotlin_ng::LANGUAGE` grammar found that
; `"break"`, `"continue"`, and `"reified"` all appear as literal *text* in
; `grammar.js` but are NOT reachable as their own visible anonymous query
; tokens in the actual compiled grammar (presumably inlined/aliased away
; during grammar generation) -- including them made `HighlightConfiguration
; ::new` fail with a real `QueryError { kind: NodeType }` the first time
; this query was tested, caught and fixed by querying the real compiled
; grammar's own symbol table instead of guessing from source text a second
; time.

(line_comment) @comment
(block_comment) @comment

(string_literal) @string
(character_literal) @string
(multiline_string_literal) @string

(number_literal) @number
(float_literal) @number

(function_declaration name: (identifier) @function)
(call_expression . (identifier) @function)
(navigation_expression (identifier) @function .)

(class_declaration name: (identifier) @type)
(user_type (identifier) @type)

[
  "fun" "val" "var" "class" "interface" "object" "enum"
  "if" "else" "when" "for" "while" "do"
  "return" "throw" "try" "catch" "finally"
  "is" "as" "in" "import" "package"
  "companion" "constructor" "init" "get" "set" "by" "where" "typealias"
  "vararg" "override" "private" "protected" "public" "internal"
  "sealed" "data" "annotation" "inner" "value" "lateinit" "const" "suspend"
  "abstract" "open" "final" "operator" "infix" "inline" "external"
  "crossinline" "noinline" "tailrec" "actual" "expect" "out" "dynamic"
  "this" "super"
] @keyword
