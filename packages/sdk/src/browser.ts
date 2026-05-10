// Browser-targeted entrypoint for `@mnemonik-xyz/sdk`.
//
// Re-exports the same public surface as `./index.ts`. The DIFFERENCE is
// at bundle time: `dist/browser.js` is produced by esbuild
// (`build/browser.config.mjs`) with a resolver shim that swaps
// `./wasm.js` → `./wasm.browser.ts`, so the published bundle contains no
// `node:*` imports and no Node-only fallbacks.
//
// Use this entry from MV3 service workers, content scripts, popups, web
// pages, and any other context where Node builtins are unavailable:
//
//   import { MnemonicClient } from "@mnemonik-xyz/sdk/browser";
//
// The default `@mnemonik-xyz/sdk` entry resolves to this file under the
// `browser` export condition (Node bundlers like webpack / rollup respect
// it automatically); other consumers can pin the `/browser` subpath
// explicitly.

export * from "./index.js";
