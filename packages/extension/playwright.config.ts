import { defineConfig } from "@playwright/test";

// T19 — extension E2E suite.
//
// All extension-loaded specs go through `tests/e2e/setup.ts`'s
// `launchExtension()` helper, which spins a persistent Chromium
// context with `--load-extension=$dist`. The T13 inline FAB/overlay
// spec (`fab-overlay.spec.ts`) still uses a default browser context
// because it injects styles inline rather than loading the built
// extension — Playwright runs both shapes happily under one config.
//
// Run locally:
//   bun run build
//   bunx playwright install chromium
//   bun run e2e
//
// CI: `.github/workflows/ext-e2e.yml` builds + installs Chromium +
// runs this config, then uploads `playwright-report/` and
// `test-results/` on failure.

export default defineConfig({
  testDir: "./tests/e2e",
  testMatch: ["**/*.spec.ts"],
  timeout: 30_000,
  expect: { timeout: 5_000 },
  // Extension-loaded tests use a persistent context with a unique
  // user-data-dir per test — they cannot share state safely. Setting
  // `workers: 1` keeps CI deterministic without relying on isolation
  // tricks inside the harness; the local-dev cost is acceptable for
  // ~8 specs.
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: {
    headless: true,
    viewport: { width: 1280, height: 720 },
    trace: "on-first-retry",
    video: "retain-on-failure",
  },
  // Single project — `--load-extension` requires Chromium specifically;
  // Firefox + WebKit don't support the MV3 loader flag.
  projects: [
    {
      name: "chromium-extension",
      use: { browserName: "chromium" },
    },
  ],
});
