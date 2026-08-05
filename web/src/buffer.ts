// Real loader for the real spartan-buffer-wasm module -- the exact same
// rope/branching-undo-tree Document engine every other Spartan UI surface
// depends on, compiled to WASM (crates/spartan-buffer-wasm, promoted from
// spikes/wasm-buffer-spike, §75.85/§75.89). This file deliberately
// imports from `./wasm-gen/`, a real, generated-not-committed directory
// -- `npm run build:wasm` (see package.json) must run at least once
// before this import resolves; see README.md.
//
// eslint-disable-next-line import/no-unresolved -- generated at build time, see above
// @ts-ignore -- ./wasm-gen only exists after `npm run build:wasm` has run
import init, { WasmDocument } from "./wasm-gen/spartan_buffer_wasm.js";

let initPromise: Promise<void> | null = null;

/** Real, one-time WASM module init -- every caller awaits the same real
 * promise, matching the same "init once, share the result" pattern the
 * real Node-side spikes already established for this same crate. */
export function ensureBufferWasmInit(): Promise<void> {
  if (!initPromise) {
    // The generated wasm-bindgen loader is typed differently depending on
    // whether its bundler glue is emitted as async or sync initialization.
    // Normalize both real shapes to the shared Promise<void> contract.
    initPromise = Promise.resolve(init()).then(() => undefined);
  }
  return initPromise;
}

export type { WasmDocument };
export { WasmDocument as Document };
