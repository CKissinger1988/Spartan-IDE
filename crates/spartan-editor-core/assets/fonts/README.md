# Bundled font: JetBrains Mono

`JetBrainsMono-Regular.ttf` and `JetBrainsMono-Bold.ttf` are the real JetBrains
Mono font (Latin subset, weights 400/700), extracted from the official
`@fontsource/jetbrains-mono` npm package (v5.2.8) by decompressing its
WOFF2 files back to plain TrueType via `fonttools` — the same real font
data, just re-encoded to a container `fontdb`/`ttf-parser` (this crate's
own font-loading stack) can parse directly; WOFF2 itself isn't supported.

Licensed under the SIL Open Font License 1.1 (`OFL-JetBrainsMono.txt`,
copied verbatim from the same package), which explicitly permits bundling
and redistribution in software. Copyright 2020 The JetBrains Mono Project
Authors (<https://github.com/JetBrains/JetBrainsMono>).

See `crates/spartan-editor-core/src/fonts.rs` for how these are embedded
and loaded into `cosmic-text`'s `FontSystem`.
