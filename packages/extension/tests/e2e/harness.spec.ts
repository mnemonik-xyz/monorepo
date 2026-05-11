// T19 — harness smoke: validates that the extension boots under
// `--load-extension=dist/` so the other specs can rely on a known-good
// launcher. Each capture/recall spec includes a similar
// `popup-mounts-without-errors` guard; centralising the boot check
// here gives a fast feedback signal when the build pre-reqs regress
// (missing icons, broken manifest, SW init crash, etc).
//
// Not a TDD anchor. If this one goes red, every other spec is going
// red too — fix the harness first.

import { expect, test } from "@playwright/test";
import { launchExtension } from "./setup.js";

test("harness_loads_extension_and_resolves_id", async () => {
  const ext = await launchExtension();
  try {
    // Extension ids are exactly 32 lowercase letters.
    expect(ext.extensionId).toMatch(/^[a-p]{32}$/);
    // The service worker registers under the same id.
    expect(ext.worker.url()).toContain(
      `chrome-extension://${ext.extensionId}/`,
    );
  } finally {
    await ext.close();
  }
});

test("harness_opens_popup_without_console_errors", async () => {
  const ext = await launchExtension();
  const errors: string[] = [];
  try {
    const page = await ext.openPopup();
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    // First-run popup defaults to Local tier — Onboarding only kicks
    // in when storage_tier === "cloud" AND there's no session. We see
    // the Capture/Recall/Verify tab row instead.
    await expect(page.getByRole("tab", { name: /^Capture$/ })).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByRole("tab", { name: /^Recall$/ })).toBeVisible();
    await expect(page.getByRole("tab", { name: /^Verify$/ })).toBeVisible();
  } finally {
    await ext.close();
  }
  // Hard-fail on any pageerror; console-error tolerance is per-spec
  // (some specs intentionally exercise error paths).
  expect(errors.filter((e) => !/Failed to fetch/i.test(e))).toEqual([]);
});
