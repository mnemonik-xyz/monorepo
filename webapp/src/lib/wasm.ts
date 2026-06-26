/**
 * Lazy loader for the `mnemonic-core` npm package — the wasm-pack
 * `--target web` build of `core/` published to npm.
 *
 * Previously the webapp ran `wasm-pack build` itself via `npm run build:wasm`
 * and consumed the output from `webapp/src/wasm/`. That meant every build
 * environment (including Cloudflare Pages) needed the Rust toolchain. Now
 * the WASM bytes ship as a normal npm dependency: `npm install` is enough.
 *
 * Vite tree-shakes the import so the WASM bytes are only fetched on routes
 * that actually need them (`/install`, `/sign/*`). The init() Promise is
 * cached so concurrent callers share a single fetch.
 */

// The published `mnemonic-core` package ships its own .d.ts which TypeScript
// resolves via `node_modules/mnemonic-core/mnemonic_core.d.ts`. We re-export
// it as `MnemonicWasm` so existing callsites that reference the type stay
// compiling — and inheriting from the published types means any future
// surface change in `core/pkg-web/` is caught at the webapp build boundary.
export type MnemonicWasm = typeof import("mnemonic-core");

let cached: Promise<MnemonicWasm> | null = null;

export function loadWasm(): Promise<MnemonicWasm> {
  if (cached) return cached;
  cached = (async () => {
    // `mnemonic-core` ships the wasm-pack `--target web` ES module. Vite
    // resolves the import from `node_modules/mnemonic-core/mnemonic_core.js`
    // and rewrites the WASM URL to a bundled asset path during build.
    const mod = await import("mnemonic-core");
    await mod.default();
    return mod;
  })();
  return cached;
}

/**
 * Reset the cached init promise — used by tests that mock the module.
 * Production code should never call this.
 */
export function __resetWasmForTests(): void {
  cached = null;
}
