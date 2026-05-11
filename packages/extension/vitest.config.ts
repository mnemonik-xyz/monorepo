import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    // node default keeps the unit tests (server / store / adapters)
    // fast; the popup component tests opt into jsdom locally.
    environment: "node",
    include: [
      "tests/unit/**/*.test.ts",
      "tests/component/**/*.test.tsx",
      "src/**/*.test.ts",
    ],
    environmentMatchGlobs: [
      ["tests/component/**/*.test.tsx", "jsdom"],
      ["tests/unit/content/**/*.test.ts", "jsdom"],
    ],
    exclude: ["tests/e2e/**", "node_modules/**", "dist/**"],
    setupFiles: ["./tests/setup.popup.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
      include: ["src/**/*.ts", "src/**/*.tsx"],
    },
  },
});
