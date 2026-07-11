// Real, thin wrapper around web-tree-sitter's real API -- no mocking, no
// stubbed grammar. Deliberately pinned to web-tree-sitter@0.20.8 (see
// README.md's "A real version-compatibility finding" section for why the
// current 0.26.x release cannot load these prebuilt grammars at all).

const Parser = require("web-tree-sitter");

let initialized = false;

/** Real, one-time WASM runtime init -- must happen before any Language.load(). */
async function ensureInit() {
  if (!initialized) {
    await Parser.init();
    initialized = true;
  }
}

/**
 * Loads a real, prebuilt language grammar (from `tree-sitter-wasms`) and
 * returns a real Parser configured to use it.
 */
async function loadParser(languageName) {
  await ensureInit();
  const wasmPath = require.resolve(`tree-sitter-wasms/out/tree-sitter-${languageName}.wasm`);
  const language = await Parser.Language.load(wasmPath);
  const parser = new Parser();
  parser.setLanguage(language);
  return { parser, language };
}

module.exports = { loadParser };
