// Vite's own ambient types, needed for its asset-URL import suffixes.
//
// `treeSitter.ts` imports real `.wasm` files with Vite's `?url` suffix so
// the grammars are emitted as real hashed build assets and referenced by
// URL at runtime, rather than being inlined into the JS bundle (they are
// hundreds of KB each, and web-tree-sitter wants a URL to fetch anyway).
// Without this reference TypeScript has no declaration for those module
// specifiers and reports TS2307.
/// <reference types="vite/client" />
