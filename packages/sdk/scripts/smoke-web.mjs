// Wave-1 smoke: verify the wasm-pack `--target web` artifact loads under the
// current runtime and exposes the expected named export.
//
// Pass criterion: prints `function`. Failure means the runtime cannot import
// the artifact at all (ESM resolution, syntax, or top-level `import.meta`).
//
// Note: this only checks the JS module surface. Actually invoking the export
// requires calling the default `init()` first (web target loads .wasm via
// fetch / fs lazily). That deeper check belongs in T2's signer-contract suite.

import * as wasm from "../../../core/pkg-web/mnemonic_core.js";

console.log(typeof wasm.sign_cose_payload);
