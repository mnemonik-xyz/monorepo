import { defineConfig, devices } from "@playwright/test";

// Playwright config — exclusively used for the T13 FAB + Recall
// overlay E2E spec. NOT wired into CI yet (no chromium download in
// the default CI image; documented gap in decisions.md). Run locally
// with `bunx playwright install chromium && bun run e2e`.
export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: ["**/*.spec.ts"],
  timeout: 30_000,
  fullyParallel: true,
  use: {
    headless: true,
    viewport: { width: 1280, height: 720 },
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
