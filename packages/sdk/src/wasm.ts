// Internal WASM loader for `mnemonic-core` (`--target web` build at
// `core/pkg-web/`). NOT re-exported in `index.ts` — only the SDK's own
// modules use it.
//
// Why a custom loader (instead of `import init, * as wasm from '...'`):
// the `--target web` artifact's default-export `init()` resolves the
// `.wasm` URL via `import.meta.url` + `fetch`. That works in browsers,
// Node 20+ (with native `fetch`), Bun, and Deno without a bundler — see
// Task 1 smoke matrix in decisions.md. We cache the initialized module
// behind a single Promise so concurrent callers share one init.
//
// The fetch path requires no `node:fs` and no `node:url` — `import.meta.url`
// is a Web API. Pure ESM, zero `node:*` imports.

// eslint-disable-next-line @typescript-eslint/consistent-type-imports
import type * as MnemonicCore from "../../../core/pkg-web/mnemonic_core.js";

let modulePromise: Promise<typeof MnemonicCore> | null = null;

/**
 * Lazily instantiate and return the WASM module.
 *
 * Subsequent calls return the same Promise — the WASM table is shared
 * across all SDK consumers in the same JS realm.
 *
 * If `WASM_OVERRIDE` is set via `__setWasmForTesting` (vitest only), it
 * replaces the dynamic import — that's the entire test-mocking surface.
 */
export async function loadWasm(): Promise<typeof MnemonicCore> {
  if (WASM_OVERRIDE) return WASM_OVERRIDE;
  if (modulePromise) return modulePromise;
  modulePromise = (async () => {
    // Dynamic import keeps the WASM lazily loaded — `import.meta.resolve`
    // would also work but is younger; dynamic specifier covers all four
    // target runtimes today.
    const mod = (await import(
      "../../../core/pkg-web/mnemonic_core.js"
    )) as typeof MnemonicCore & {
      default: (input?: unknown) => Promise<unknown>;
    };
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

let WASM_OVERRIDE: typeof MnemonicCore | null = null;

/** @internal — tests only. Pass `null` to clear. */
export function __setWasmForTesting(mock: typeof MnemonicCore | null): void {
  WASM_OVERRIDE = mock;
  // Reset the cached promise so the next loadWasm() picks up the new state.
  modulePromise = null;
}
