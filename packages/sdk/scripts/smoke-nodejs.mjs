// Wave-1 smoke: verify the wasm-pack `--target nodejs` artifact loads under
// the current runtime and exposes the expected named export.
//
// nodejs target emits CommonJS — under `.mjs` we use dynamic import which Node
// auto-wraps the CJS exports. Bun and Deno also support CJS interop, but this
// path is the most fragile across runtimes.

import wasmModule from "../../../core/pkg-nodejs/mnemonic_core.js";

const fn = wasmModule?.sign_cose_payload ?? wasmModule?.default?.sign_cose_payload;
console.log(typeof fn);
