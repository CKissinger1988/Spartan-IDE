# Bundled font: JetBrains Mono

Same real, OFL-licensed TTF files as `crates/spartan-editor-core/assets/fonts/`
and `desktop/`/`web/`'s `@fontsource/jetbrains-mono` webfont — see that
crate's own `assets/fonts/README.md` for exactly how these were produced.

Registered via the `expo-font` config plugin in `app.json`, which bundles
them as real native font resources at build time — no runtime `useFonts()`
loading flicker. The resulting `fontFamily` name is the file's own
basename: `"JetBrainsMono-Regular"` / `"JetBrainsMono-Bold"`.
