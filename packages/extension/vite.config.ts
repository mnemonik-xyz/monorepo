import { defineConfig } from "vite";
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
