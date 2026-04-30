// Internal WASM loader for `mnemonic-core` (`--target web` build). NOT
// re-exported in `index.ts` — only the SDK's own modules use it.
//
// Why a custom loader (instead of `import init, * as wasm from '...'`):
// the `--target web` artifact's default-export `init()` resolves the
// `.wasm` URL via `import.meta.url` + `fetch`. That works in browsers,
// Node 20+ (with native `fetch`), Bun, and Deno without a bundler — see
// Task 1 smoke matrix in decisions.md. We cache the initialized module
// behind a single Promise so concurrent callers share one init.
//
// Packaging note (npm): the WASM artifact is mirrored into the SDK's
// `dist/wasm/` directory by `scripts/build-wasm.sh`, so it ships inside
// the published tarball (the `files: ["dist", ...]` allowlist in
// `package.json`). At runtime we resolve the artifact relative to the
// COMPILED `dist/wasm.js` via `new URL("./wasm/mnemonic_core.js",
// import.meta.url)`. That URL form works identically under Node 20+,
// Bun, Deno, browsers, and Cloudflare Workers — no `node:*` imports
// required, no bundler required.
//
// Why not a static `import` specifier:
// - The published path (`./wasm/mnemonic_core.js` from `dist/wasm.js`)
//   does not match the source-side path (`./wasm/mnemonic_core.js` from
//   `src/wasm.ts` — file does not exist in `src/`). Tests bypass the
//   dynamic-import path entirely via `__setWasmForTesting`, so the
//   non-existent source-side path is never hit during `vitest run`.
//   See `test/helpers/wasm-mock.ts`.
// - `new URL(specifier, import.meta.url)` defers resolution to runtime,
//   which means the only place the path needs to exist is `dist/wasm/`.

// Local TypeScript only needs the surface type — the real declaration file
// is shipped at `dist/wasm/mnemonic_core.d.ts`. We declare a minimal shape
// inline rather than importing the relative `.d.ts` so that source-side
// `tsc -b` does not require the WASM artifact to exist before
// `build:wasm` runs.
interface MnemonicCoreModule {
  default: (input?: unknown) => Promise<unknown>;
  generate_keypair: () => unknown;
  sign_challenge: (kp: unknown, bytes: Uint8Array) => Uint8Array;
  sign_cose_payload: (payload: Uint8Array, kp: unknown) => Uint8Array;
  import_keypair_json: (s: string) => unknown;
  export_keypair_json: (kp: unknown) => string;
}

let modulePromise: Promise<MnemonicCoreModule> | null = null;

/**
 * Lazily instantiate and return the WASM module.
 *
 * Subsequent calls return the same Promise — the WASM table is shared
 * across all SDK consumers in the same JS realm.
 *
 * If `WASM_OVERRIDE` is set via `__setWasmForTesting` (vitest only), it
 * replaces the dynamic import — that's the entire test-mocking surface.
 */
export async function loadWasm(): Promise<MnemonicCoreModule> {
  if (WASM_OVERRIDE) return WASM_OVERRIDE;
  if (modulePromise) return modulePromise;
  modulePromise = (async () => {
    // Resolve the published artifact path relative to the compiled
    // `dist/wasm.js`. At runtime under Node/Bun/Deno/browsers, this
    // produces a URL pointing at `dist/wasm/mnemonic_core.js`, which
    // ships inside the npm tarball.
    const url = new URL("./wasm/mnemonic_core.js", import.meta.url);
    const mod = (await import(url.href)) as MnemonicCoreModule;
    // The `--target web` build needs an explicit `init()` call before any
    // export is invoked. Without it, the WASM memory is uninitialized and
    // every call panics with `recursive use of an object`.
    await mod.default();
    return mod;
  })();
  return modulePromise;
}

// --------------------------------------------------------------------------
// Test-only override hook
// --------------------------------------------------------------------------
//
// The SDK's test files don't want the real WASM loaded — they want a mock
// surface so they can assert "did the SDK call sign_cose_payload with these
// bytes?". We expose a single setter the tests use to swap in a mock; in
// production the override is `undefined` and the dynamic import path runs.
//
// Marked with double-underscore prefix to discourage application-level use.

let WASM_OVERRIDE: MnemonicCoreModule | null = null;

/** @internal — tests only. Pass `null` to clear. */
export function __setWasmForTesting(mock: MnemonicCoreModule | null): void {
  WASM_OVERRIDE = mock;
  // Reset the cached promise so the next loadWasm() picks up the new state.
  modulePromise = null;
}
