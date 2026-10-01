// Browser-only WASM loader. Public surface mirrors `./wasm.ts` so that
// `cose.ts`, `keypair.ts`, and `signer.ts` can be reused unchanged — at
// bundle time the esbuild shim in `build/browser.config.mjs` rewrites
// `./wasm.js` imports inside `dist/browser.js` to resolve THIS file.
//
// What's different vs. `./wasm.ts`:
//   - No `node:fs/promises` import. Browsers can't resolve `node:*` and the
//     T02 static-analysis test (`tests/browser.test.ts::no_node_imports`)
//     fails the build if the bundle leaks one.
//   - No `globalThis.self` shim — service workers / regular pages already
//     have `self` defined.
//   - `default()` is invoked with no arguments, letting wasm-bindgen's
//     `--target web` init resolve the `.wasm` via `fetch(import.meta.url)`
//     over HTTP(S). The Chrome extension serves `wasm/mnemonic_core_bg.wasm`
//     from its packaged bundle origin — no `file://` quirks.

import type { MnemonicCoreModule } from "./wasm.js";

let modulePromise: Promise<MnemonicCoreModule> | null = null;

/**
 * Lazily instantiate the WASM module under a browser runtime.
 *
 * Subsequent calls return the same Promise — the WASM table is shared
 * across all SDK consumers in the same JS realm.
 *
 * If `WASM_OVERRIDE` is set via `__setWasmForTesting`, it replaces the
 * dynamic import — that's the entire test-mocking surface.
 */
export async function loadWasm(): Promise<MnemonicCoreModule> {
  if (WASM_OVERRIDE) return WASM_OVERRIDE;
  if (modulePromise) return modulePromise;
  modulePromise = (async () => {
    const wasmJsUrl = new URL("./wasm/mnemonic_core.js", import.meta.url);
    const mod = (await import(wasmJsUrl.href)) as MnemonicCoreModule;
    if (typeof mod.default === "function") {
      await mod.default();
    }
    return mod;
  })();
  return modulePromise;
}

let WASM_OVERRIDE: MnemonicCoreModule | null = null;

/** @internal — tests only. Pass `null` to clear. */
export function __setWasmForTesting(mock: MnemonicCoreModule | null): void {
  WASM_OVERRIDE = mock;
  modulePromise = null;
}
