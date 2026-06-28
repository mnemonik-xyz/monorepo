import { test, expect, type Page } from "@playwright/test";
import { mcpBase } from "./_helpers";

/**
 * Cross-route smoke for the Evidence Ledger surface (Task 6, audit F1): each new
 * public route renders its key content and the masthead nav links move between
 * them. The pages are designed to degrade to deterministic sample data when no
 * live node answers (Decision 2), so we abort every MCP backend request up front
 * — the assertions then hold regardless of whether a real backend is reachable,
 * which is exactly the no-backend resilience the ledger surface promises.
 */

/** Force the sample-fallback path by failing all calls to the MCP backend. */
async function stubBackendOffline(page: Page): Promise<void> {
  await page.route(`${mcpBase()}/**`, (route) => route.abort());
}

test.describe("Evidence ledger surface (smoke)", () => {
  test.beforeEach(async ({ page }) => {
    await stubBackendOffline(page);
  });

  test("/ledger renders recalled artifacts with the sample banner", async ({
    page,
  }) => {
    await page.goto("/ledger");
    await expect(
      page.getByRole("heading", { name: "Recalled artifacts on the node." }),
    ).toBeVisible();
    // No backend -> the clearly-labeled sample banner + at least one artifact card.
    await expect(page.getByTestId("sample-banner")).toBeVisible();
    await expect(page.getByRole("listitem").first()).toBeVisible();
  });

  test("/analytics renders the attestation timeline chart", async ({
    page,
  }) => {
    await page.goto("/analytics");
    await expect(
      page.getByRole("heading", { name: "Attestations over time." }),
    ).toBeVisible();
    await expect(page.getByTestId("timeline-chart")).toBeVisible();
  });

  test("/blog lists posts and opens one", async ({ page }) => {
    await page.goto("/blog");
    await expect(
      page.getByRole("heading", { name: "Blog", level: 1 }),
    ).toBeVisible();

    const firstPost = page.getByRole("link", { name: /trustless/i }).first();
    await expect(firstPost).toBeVisible();
    await firstPost.click();

    await expect(page).toHaveURL(/\/blog\/.+/);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  });

  test("/blog/:slug renders a single post by slug", async ({ page }) => {
    await page.goto("/blog/verifiable-memory-for-agents");
    await expect(
      page.getByRole("heading", {
        name: /trustless agents need trustless memory/i,
      }),
    ).toBeVisible();
  });

  test("header nav moves between ledger, analytics, and blog", async ({
    page,
  }) => {
    // Nav links expose their srLabel via aria-label, so address them by that.
    await page.goto("/ledger");

    await page.getByRole("link", { name: "Attestation analytics" }).click();
    await expect(page).toHaveURL(/\/analytics$/);
    await expect(
      page.getByRole("heading", { name: "Attestations over time." }),
    ).toBeVisible();

    await page.getByRole("link", { name: "Protocol blog" }).click();
    await expect(page).toHaveURL(/\/blog$/);
    await expect(
      page.getByRole("heading", { name: "Blog", level: 1 }),
    ).toBeVisible();

    await page.getByRole("link", { name: "Evidence ledger" }).click();
    await expect(page).toHaveURL(/\/ledger$/);
    await expect(
      page.getByRole("heading", { name: "Recalled artifacts on the node." }),
    ).toBeVisible();
  });
});
