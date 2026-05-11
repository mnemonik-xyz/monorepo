import { defineConfig } from "vite";
import { resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { crx } from "@crxjs/vite-plugin";
import manifest from "./manifest.json" with { type: "json" };

export default defineConfig({
  plugins: [react(), crx({ manifest })],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
    target: "es2022",
    rollupOptions: {
      // The crxjs plugin auto-discovers entries from manifest.json.
      // We add `test-helpers.ts` as an explicit input so rollup
      // emits it as its own chunk that retains the full set of
      // re-exported `key-escrow` symbols (`wrapSecret`, `unwrapSecret`,
      // `uploadEscrow`, …). Production code paths never reference
      // this entry; it exists solely so the Playwright e2e harness
      // (`restore-on-second-profile.spec.ts`) can dynamic-import the
      // hashed chunk URL and call the exports by name without being
      // affected by rollup's per-export tree-shaking of the main
      // `key-escrow` chunk.
      input: {
        "auth-test-helpers": resolve(
          import.meta.dirname,
          "src/auth/test-helpers.ts",
        ),
      },
      output: {
        // Force `test-helpers.ts` into its own named chunk so the e2e
        // spec can resolve it deterministically via the
        // `assets/auth-test-helpers-*.js` glob.
        manualChunks(id: string) {
          if (id.includes("src/auth/test-helpers")) return "auth-test-helpers";
          return undefined;
        },
        // Preserve external export names verbatim so the e2e harness
        // can call `m.wrapSecret(...)` on the dynamic-imported
        // namespace object instead of guessing rollup's minified
        // aliases (`m.pA`, …).
        minifyInternalExports: false,
      },
    },
  },
  // Workers must be ESM so rollup can code-split the embedder bundle
  // (T04 spawns the embedder via `new Worker(new URL(...), { type:
  // "module" })`; iife — the rollup default — cannot host dynamic
  // chunks).
  worker: {
    format: "es",
  },
  server: {
    port: 5174,
    strictPort: true,
    hmr: {
      port: 5175,
    },
  },
});
