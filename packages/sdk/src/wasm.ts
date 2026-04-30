// Internal WASM loader for `mnemonic-core`. Loads the SINGLE published
// `--target web` artifact and initializes it correctly on every host.
// NOT re-exported in `index.ts` — only the SDK's own modules use it.
//
// History (decisions.md):
//   - 0.1.0: shipped `--target web` only; broken on Node because
//     `fetch(file://)` crashes inside undici.
//   - 0.1.1: shipped BOTH `--target web` and `--target nodejs`; runtime
//     detect picked one. The `--target nodejs` artifact crashes at
//     module-eval on Node 20+ macOS with
//     `WebAssembly.Table.grow(): failed to grow table by 4` — a known
//     wasm-bindgen issue with `--reference-types` table init. Inconsistent
//     across envs (worked in CI, failed on fresh user installs).
//   - 0.1.2 (this file): ship ONLY `--target web`. On the browser path use
//     `default()` (its own `fetch(import.meta.url)` over HTTPS — fine).
//     On Node/Bun/Deno read the `.wasm` file from disk via
//     `node:fs/promises.readFile` and pass the bytes directly to
//     `default(bytes)` — `--target web`'s init function accepts BufferSource
//     and skips the broken `fetch(file://)` path entirely. Single artifact,
//     smaller tarball, no CJS/ESM mode-flip subdir.
//
// We cache the initialized module behind a single Promise so concurrent
// callers share one init.
//
// Packaging note (npm): the WASM artifact is mirrored into `dist/wasm/`
// by `scripts/build-wasm.sh` and ships inside the published tarball
// (`files: ["dist", ...]` in `package.json`).
//
// Why not a static `import` specifier:
// - The published path (`./wasm/mnemonic_core.js` from `dist/wasm.js`) does
//   not exist on the source side. Tests bypass the dynamic-import path
//   entirely via `__setWasmForTesting`, so the non-existent source-side
//   path is never hit during `vitest run`. See `test/helpers/wasm-mock.ts`.
// - `new URL(specifier, import.meta.url)` defers resolution to runtime,
//   which means the only place the path needs to exist is `dist/wasm/`.

// Local TypeScript only needs the surface type — the real declaration file
// is shipped at `dist/wasm/mnemonic_core.d.ts`. We declare a minimal shape
// inline rather than importing the relative `.d.ts` so that source-side
// `tsc -b` does not require the WASM artifact to exist before
// `build:wasm` runs.
interface MnemonicCoreModule {
  // `--target web` exports a `default` init function. With no argument it
  // resolves the .wasm via `fetch(import.meta.url)`; with a BufferSource
  // argument it instantiates from those bytes directly (used on Node/Bun/Deno
  // to bypass the broken `fetch(file://)` path).
  default?: (input?: unknown) => Promise<unknown>;
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
    // Always import the SAME `--target web` artifact. It's a thin JS
    // module that exports a `default()` initializer accepting an optional
    // BufferSource — we use that to skip the broken `fetch(file://)` path
    // on Node/Bun/Deno.
    const wasmJsUrl = new URL("./wasm/mnemonic_core.js", import.meta.url);
    const mod = (await import(wasmJsUrl.href)) as MnemonicCoreModule;

    // Host detection. Anything that's NOT a browser (no `window` +
    // `window.document`) gets the fs-shim path: Node, Bun, Deno via their
    // Node-compat surfaces, Cloudflare Workers (rare for an SDK consumer
    // but covered by the early-return on `default()`).
    //
    // `globalThis as any` to avoid pulling in `@types/node` here; the SDK
    // ships zero runtime deps and we want the type-check footprint small.
    const g = globalThis as unknown as {
      window?: { document?: unknown };
    };
    const isBrowser =
      typeof g.window !== "undefined" &&
      typeof g.window.document !== "undefined";

    if (typeof mod.default !== "function") {
      // Defensive: `--target web` always exports default(). If a custom
      // mock somehow ends up here it can omit init.
      return mod;
    }

    if (isBrowser) {
      // Browser: `default()` resolves the .wasm via `fetch(import.meta.url)`
      // over HTTPS — fine.
      await mod.default();
    } else {
      // Node / Bun / Deno: read the .wasm file from disk and pass bytes
      // directly to `default(bytes)`. Bypasses the broken `fetch(file://)`
      // path entirely. `--target web`'s init accepts BufferSource.
      //
      // Self-shim: the Rust `getrandom` 0.2 (`js` feature) backend probes
      // `self.crypto.getRandomValues` first and only falls back to
      // `require('crypto').randomFillSync` when that lookup throws. In Node
      // 20+ ESM, `globalThis.crypto` exists but `self` is NOT a global, so
      // the WebCrypto probe fails and the fallback kicks in — which then
      // crashes with `arg0.require is not a function` because we're not in
      // CJS so there is no `module.require`. Patching `globalThis.self =
      // globalThis` (well-known wasm-bindgen-on-Node workaround) makes the
      // WebCrypto branch resolve correctly. Idempotent, scoped to
      // initialization. Bun and Deno already define `self` as a global, so
      // this is a no-op for them.
      const gt = globalThis as unknown as { self?: unknown };
      if (typeof gt.self === "undefined") {
        gt.self = globalThis;
      }
      const wasmBgUrl = new URL(
        "./wasm/mnemonic_core_bg.wasm",
        import.meta.url
      );
      const fs = (await import("node:fs/promises")) as {
        readFile: (p: URL) => Promise<Uint8Array>;
      };
      const bytes = await fs.readFile(wasmBgUrl);
      // Pass as `{ module_or_path: bytes }` rather than positional bytes —
      // the positional form prints a deprecation warning to stderr in
      // wasm-bindgen ≥ 0.2.95.
      await mod.default({ module_or_path: bytes });
    }
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
