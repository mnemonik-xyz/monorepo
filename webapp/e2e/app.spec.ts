import { test, expect } from "@playwright/test";

test("homepage renders heading", async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Verifiable memory for AI agents." }),
  ).toBeVisible();
});
