// Wave-1 smoke: verify the wasm-pack `--target bundler` artifact loads under
// the current runtime and exposes the expected named export.
//
// The bundler target uses synchronous `import * as wasm from "./*.wasm"` —
// this only works inside a bundler (Vite/esbuild/webpack). Running it under
// raw Node/Bun/Deno is expected to fail; the smoke confirms the failure mode.

import * as wasm from "../../../core/pkg-bundler/mnemonic_core.js";

console.log(typeof wasm.sign_cose_payload);
