import { test, expect } from "@playwright/test";
import { mcpBase } from "./_helpers";

/**
 * Smoke coverage for the `/install` page:
 *   - Identity panel renders generate / import / export / clear buttons
 *   - Clicking "Generate keypair" runs the WASM `generate_keypair`,
 *     persists `mnemonic.identity` to localStorage, and renders the
 *     base58 pubkey
 *   - The Cursor / VS Code / Claude.ai install buttons emit deeplinks
 *     in the formats the respective clients accept (per
 *     `InstallButtons.tsx::cursorDeeplink/vscodeDeeplink`)
 */

test.describe("/install page", () => {
  test("renders landing → install navigation", async ({ page }) => {
    await page.goto("/");
    // Landing page CTAs lead to /install.
    const cta = page.getByRole("link", { name: /get started/i }).first();
    await expect(cta).toBeVisible();
    await cta.click();
    await expect(page).toHaveURL(/\/install$/);
  });

  test("identity panel: generate keypair populates pubkey + DID", async ({
    page,
  }) => {
    await page.goto("/install");

    // The IdentityPanel button labelled "Generate keypair".
    const generate = page.getByTestId("identity-generate");
    await expect(generate).toBeVisible();
    await generate.click();

    // After generate, the pubkey + DID should be populated.
    const pubkey = page.getByTestId("identity-pubkey");
    await expect(pubkey).toBeVisible({ timeout: 10_000 });
    const pubkeyText = (await pubkey.textContent())?.trim() ?? "";
    expect(pubkeyText.length).toBeGreaterThan(30); // base58 Ed25519 pubkey > 30 chars.

    // localStorage now has the keypair JSON in the canonical shape.
    const stored = await page.evaluate(() =>
      localStorage.getItem("mnemonic.identity")
    );
    expect(stored).toBeTruthy();
    const parsed = JSON.parse(stored as string);
    expect(parsed).toEqual(
      expect.objectContaining({
        secret: expect.any(Array),
        pubkey_base58: pubkeyText,
      })
    );
    expect(parsed.secret.length).toBe(64); // Solana keypair: 32-byte seed + 32-byte pubkey.
  });

  test("install buttons emit valid deeplinks", async ({ page }) => {
    await page.goto("/install");

    // Cursor — base64 config payload + name=Mnemonic.
    const cursorHref = await page
      .getByTestId("install-cursor")
      .getAttribute("href");
    expect(cursorHref).toMatch(
      /^cursor:\/\/anysphere\.cursor-deeplink\/mcp\/install\?name=Mnemonic&config=/
    );
    // Decode the base64 payload — should match the Cursor MCP HTTP shape.
    const cursorB64 = new URL(cursorHref!).searchParams.get("config")!;
    const cursorCfg = JSON.parse(atob(cursorB64));
    expect(cursorCfg.url).toContain("/mcp");
    expect(cursorCfg.type).toBe("http");

    // VS Code — single URL-encoded JSON config.
    const vscodeHref = await page
      .getByTestId("install-vscode")
      .getAttribute("href");
    expect(vscodeHref).toMatch(/^vscode:mcp\/install\?/);
    const vscodeCfg = JSON.parse(
      decodeURIComponent(vscodeHref!.replace("vscode:mcp/install?", ""))
    );
    expect(vscodeCfg).toEqual(
      expect.objectContaining({
        name: "Mnemonic",
        type: "http",
        url: expect.stringContaining("/mcp"),
      })
    );

    // Claude.ai — opens a modal with the paste URL.
    await page.getByTestId("install-claude-ai").click();
    const pasteUrl = await page.getByTestId("claude-paste-url").textContent();
    expect(pasteUrl?.trim()).toMatch(/^mcp\.[\w.-]+$/);
  });
});

/**
 * Live-server smoke for the OAuth metadata endpoints. The webapp dev
 * server runs locally; this hits the configured MCP backend (defaults to
 * production) to ensure the metadata that VS Code / Claude.ai probe
 * before connector install is reachable + well-formed.
 */
test.describe("MCP server discovery (smoke against backend)", () => {
  test("/.well-known/oauth-authorization-server is RFC 8414 compliant", async ({
    request,
  }) => {
    const resp = await request.get(
      `${mcpBase()}/.well-known/oauth-authorization-server`
    );
    expect(resp.status()).toBe(200);
    const body = await resp.json();
    expect(body).toEqual(
      expect.objectContaining({
        issuer: expect.stringContaining(mcpBase()),
        authorization_endpoint: expect.stringContaining("/oauth/authorize"),
        token_endpoint: expect.stringContaining("/oauth/token"),
        registration_endpoint: expect.stringContaining("/oauth/register"),
        response_types_supported: expect.arrayContaining(["code"]),
        code_challenge_methods_supported: expect.arrayContaining(["S256"]),
      })
    );
  });

  test("/.well-known/oauth-protected-resource matches MCP spec", async ({
    request,
  }) => {
    const resp = await request.get(
      `${mcpBase()}/.well-known/oauth-protected-resource`
    );
    expect(resp.status()).toBe(200);
    const body = await resp.json();
    expect(body.resource).toContain(mcpBase());
    expect(body.bearer_methods_supported).toContain("header");
  });

  test("/health returns 200", async ({ request }) => {
    const resp = await request.get(`${mcpBase()}/health`);
    expect(resp.status()).toBe(200);
  });

  test("anonymous tools/call returns 401", async ({ request }) => {
    const resp = await request.post(`${mcpBase()}/mcp`, {
      data: {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: {
          name: "mnemonic_recall",
          arguments: { query: "anything" },
        },
      },
    });
    expect(resp.status()).toBe(401);
  });

  test("anonymous tools/list returns 200 (allowlist)", async ({ request }) => {
    const resp = await request.post(`${mcpBase()}/mcp`, {
      data: { jsonrpc: "2.0", id: 1, method: "tools/list" },
    });
    expect(resp.status()).toBe(200);
    const body = await resp.json();
    expect(body.result?.tools).toBeInstanceOf(Array);
    expect(body.result.tools.length).toBe(5);
  });

  test("notifications/* returns 202 Accepted", async ({ request }) => {
    const resp = await request.post(`${mcpBase()}/mcp`, {
      data: {
        jsonrpc: "2.0",
        method: "notifications/initialized",
        params: {},
      },
    });
    expect(resp.status()).toBe(202);
  });
});
