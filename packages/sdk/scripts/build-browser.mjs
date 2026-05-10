// esbuild configuration for the browser bundle (`dist/browser.js`).
//
// Why esbuild and not just tsc:
//   - tsc emits one .js file per source file with relative imports
//     intact. That's fine for the Node ESM consumer, but a Chrome MV3
//     service worker can't follow `./wasm.js` → `node:fs/promises`.
//   - We need ONE bundled ESM file, minified, with the WASM loader's
//     Node fallback swapped for the browser-only variant.
//
// Plugin: `wasm-browser-shim` rewrites `./wasm.js` imports (any depth)
// to `./wasm.browser.ts` co-located in the same source directory. This
// is the entire mechanism that strips `node:fs/promises` from the
// bundle — `wasm.browser.ts` doesn't import it.
//
// What we keep external:
//   - The WASM artifact itself (`./wasm/mnemonic_core.js`) is loaded at
//     runtime via `new URL(..., import.meta.url)` + dynamic `import()`.
//     esbuild leaves dynamic imports of computed expressions alone, so
//     no special config is needed; the artifact resolves relative to
//     wherever `dist/browser.js` is served from.

import * as esbuild from "esbuild";
import { stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Lives next to `build-wasm.sh` rather than under `build/` because the
// repo-root `.gitignore` excludes `build/` directories at any depth.
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");

/**
 * Redirect every relative `./wasm.js` import (or `../wasm.js`, etc.) to
 * its `wasm.browser.ts` sibling, when one exists. Plain `import('wasm.js')`
 * specifiers and unrelated paths are left untouched.
 */
const wasmBrowserShim = {
  name: "wasm-browser-shim",
  setup(build) {
    build.onResolve({ filter: /(^|\/)wasm\.(js|ts)$/ }, async (args) => {
      // Only rewrite relative imports — leaves any package-level alias
      // alone (defence in depth: today the SDK has no such import).
      if (!args.path.startsWith("./") && !args.path.startsWith("../")) {
        return null;
      }
      const candidate = path.resolve(
        args.resolveDir,
        args.path.replace(/wasm\.(js|ts)$/, "wasm.browser.ts"),
      );
      try {
        await stat(candidate);
        return { path: candidate };
      } catch {
        return null;
      }
    });
  },
};

await esbuild.build({
  entryPoints: [path.join(root, "src/browser.ts")],
  outfile: path.join(root, "dist/browser.js"),
  format: "esm",
  platform: "browser",
  target: "es2022",
  bundle: true,
  minify: true,
  legalComments: "none",
  sourcemap: false,
  treeShaking: true,
  plugins: [wasmBrowserShim],
  // No `external` — every dependency must inline. The WASM artefact is
  // loaded via `new URL(..., import.meta.url)` + dynamic import, which
  // esbuild leaves as a runtime resolution.
});

console.log("✓ packages/sdk/dist/browser.js built");
