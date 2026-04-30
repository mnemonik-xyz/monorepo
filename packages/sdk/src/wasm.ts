// Internal WASM loader for `mnemonic-core`. Picks the right wasm-pack
// artifact at runtime based on the host environment. NOT re-exported in
// `index.ts` — only the SDK's own modules use it.
//
// Why two artifacts (0.1.1 hotfix — see T15 + decisions.md "SDK 0.1.1"):
//   - `--target web` uses `fetch(import.meta.url)` to load the .wasm.
//     Browsers happy. Node 20+/22 undici `fetch()` does NOT support
//     `file://` URLs and crashes (cannot find native binding). Bun/Deno
//     hit a related WebAssembly.Table.grow() error. EVERY WASM-touching
//     CLI command was broken on all 3 target runtimes.
//   - `--target nodejs` emits CJS-shaped JS that loads the .wasm via
//     `fs.readFileSync` at module-eval. No fetch, no URL resolution —
//     works under Node, Bun, Deno without a shim.
//
// Strategy: detect host at runtime. If `window` is undefined and we look
// like Node (process.versions.node), pick `./wasm/nodejs/`. Otherwise
// (browsers, Cloudflare Workers via bundler) pick `./wasm/web/`.
//
// We cache the initialized module behind a single Promise so concurrent
// callers share one init.
//
// Packaging note (npm): both WASM artifacts are mirrored into
// `dist/wasm/{web,nodejs}/` by `scripts/build-wasm.sh` and ship inside
// the published tarball (`files: ["dist", ...]` in `package.json`).
//
// Why not a static `import` specifier:
// - The published paths (`./wasm/{web,nodejs}/mnemonic_core.js` from
//   `dist/wasm.js`) do not exist on the source side. Tests bypass the
//   dynamic-import path entirely via `__setWasmForTesting`, so the
//   non-existent source-side paths are never hit during `vitest run`.
//   See `test/helpers/wasm-mock.ts`.
// - `new URL(specifier, import.meta.url)` defers resolution to runtime,
//   which means the only place the path needs to exist is `dist/wasm/`.

// Local TypeScript only needs the surface type — the real declaration file
// is shipped at `dist/wasm/mnemonic_core.d.ts`. We declare a minimal shape
// inline rather than importing the relative `.d.ts` so that source-side
// `tsc -b` does not require the WASM artifact to exist before
// `build:wasm` runs.
interface MnemonicCoreModule {
  // `--target web` exports a `default` init function. `--target nodejs`
  // does not — WASM is initialized synchronously at module-eval via
  // `fs.readFileSync`. We optional-chain the call below.
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
    // Runtime environment detection. We treat anything that has a
    // Node-shaped `process.versions.node` AND no `window` as a Node-like
    // host (covers Node, Bun, Deno via their Node-compat surfaces). All
    // other hosts — browsers, Cloudflare Workers, Web Workers — get the
    // `--target web` artifact. See file header for why.
    //
    // `globalThis as any` to avoid pulling in `@types/node` here; the SDK
    // ships zero runtime deps and we want the type-check footprint small.
    const g = globalThis as unknown as {
      process?: { versions?: { node?: string } };
      window?: unknown;
    };
    const isNodeLike =
      typeof g.window === "undefined" &&
      typeof g.process !== "undefined" &&
      typeof g.process.versions?.node === "string";
    const subdir = isNodeLike ? "nodejs" : "web";
    // Resolve the published artifact path relative to the compiled
    // `dist/wasm.js`. At runtime under Node/Bun/Deno/browsers, this
    // produces a URL pointing at `dist/wasm/{web,nodejs}/mnemonic_core.js`,
    // which ships inside the npm tarball.
    const url = new URL(`./wasm/${subdir}/mnemonic_core.js`, import.meta.url);
    const mod = (await import(url.href)) as MnemonicCoreModule;
    // `--target web` exposes an explicit `default()` init that must be
    // awaited before any export is invoked. `--target nodejs` initializes
    // synchronously at module-eval and does not export `default`. Call
    // it conditionally.
    if (typeof mod.default === "function") {
      await mod.default();
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
