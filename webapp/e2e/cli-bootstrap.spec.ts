import { test, expect } from "@playwright/test";
import { mcpBase } from "./_helpers";

/**
 * E2E coverage for the "Send to CLI" flow on `/install` (Decision 7,
 * tasks/7.md TDD anchor `issued_ticket_redeemable_once`).
 *
 * Most of these checks require a live MCP backend (the `/api/cli-bootstrap/*`
 * endpoints are server-side). The first test stubs `fetch` in the page so it
 * runs offline; the second talks to the real server and is skipped via
 * `PLAYWRIGHT_SKIP_BACKEND_E2E=1` when the backend is unreachable.
 *
 * The webapp's MCP base URL is read from `VITE_MCP_BASE` at build time, so
 * the dev server picks up the same value the test will hit. See
 * `e2e/_helpers.ts::mcpBase`.
 */

test.describe("/install — Send to CLI (offline, fetch-stubbed)", () => {
  test("renders ticket paste-command after click", async ({ page }) => {
    // Stub the bootstrap-issue endpoint inside the page before navigation
    // so the React component sees a deterministic 200 response without a
    // live backend.
    const ticketId = "550e8400-e29b-41d4-a716-446655440000";
    await page.route("**/api/cli-bootstrap/issue", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ticket_id: ticketId }),
      });
    });

    await page.goto("/install");

    // Generate a webapp identity so the "Send to CLI" button enables.
    await page.getByTestId("identity-generate").click();
    await expect(page.getByTestId("identity-pubkey")).toBeVisible({
      timeout: 10_000,
    });

    // Seed a JWT in localStorage so the click handler attaches the Bearer
    // header instead of redirecting to /oauth/consent.
    await page.evaluate(() => {
      localStorage.setItem("mnemonic.jwt", "test-jwt-e2e");
    });

    await page.getByTestId("identity-send-to-cli").click();

    // Assert the code block contains the expected paste-command.
    const command = page.getByTestId("identity-cli-command");
    await expect(command).toBeVisible();
    await expect(command).toHaveText(
      `mnemonic identity import --ticket ${ticketId}`
    );
  });
});

test.describe("/install — Send to CLI (live backend)", () => {
  // Skip when the test runner cannot reach a real MCP server (CI without
  // backend access, fully-offline dev). Set the env to "1" to opt out.
  test.skip(
    () => !!process.env.PLAYWRIGHT_SKIP_BACKEND_E2E,
    "PLAYWRIGHT_SKIP_BACKEND_E2E=1 set — skipping live-backend redeem check"
  );

  test("issued ticket is redeemable exactly once via /redeem", async ({
    page,
    request,
  }) => {
    // Live test: the webapp obtains a real JWT via /oauth/token. For the
    // hackathon scope we treat "no JWT in localStorage" as a skip rather
    // than implementing the full OAuth dance inside Playwright. A future
    // task can wire the headless OAuth helper from the SDK.
    test.skip(
      !process.env.PLAYWRIGHT_TEST_JWT,
      "PLAYWRIGHT_TEST_JWT not set — cannot exercise live /api/cli-bootstrap/issue"
    );

    await page.goto("/install");
    await page.getByTestId("identity-generate").click();
    await expect(page.getByTestId("identity-pubkey")).toBeVisible({
      timeout: 10_000,
    });

    const jwt = process.env.PLAYWRIGHT_TEST_JWT!;
    await page.evaluate((token) => {
      localStorage.setItem("mnemonic.jwt", token);
    }, jwt);

    await page.getByTestId("identity-send-to-cli").click();

    const commandLocator = page.getByTestId("identity-cli-command");
    await expect(commandLocator).toBeVisible({ timeout: 15_000 });
    const commandText = (await commandLocator.textContent())?.trim() ?? "";
    const match = commandText.match(
      /mnemonic identity import --ticket ([0-9a-f-]{36})$/i
    );
    expect(match, `command text was: ${commandText}`).not.toBeNull();
    const ticket = match![1];

    // Redeem once — must return 200 with `{secret: number[64], pubkey_base58}`.
    const r1 = await request.get(
      `${mcpBase()}/api/cli-bootstrap/redeem/${ticket}`
    );
    expect(r1.status()).toBe(200);
    const body = await r1.json();
    expect(Array.isArray(body.secret)).toBe(true);
    expect(body.secret.length).toBe(64);
    expect(typeof body.pubkey_base58).toBe("string");

    // Second redeem must 404 (single-use; replay is indistinguishable from
    // not-found per Decision 7's uniform error surface).
    const r2 = await request.get(
      `${mcpBase()}/api/cli-bootstrap/redeem/${ticket}`
    );
    expect(r2.status()).toBe(404);
  });
});
