import { test, expect, type Page } from "@playwright/test";
import { mcpBase, randomHex, pkceChallenge } from "./_helpers";

// Serial + no retries — same /oauth/* + /mcp rate-limit reasoning as
// oauth-flow.spec.ts. Each test mints a fresh JWT (4 /oauth/* hits per
// handshake) and burns sign_memory / recall budget; running parallel +
// 2 retries against the live backend immediately blows the per-IP cap.
test.describe.configure({ mode: "serial", retries: 0 });

/**
 * The big one — full Decision-12 deferred-signing round trip from the
 * AI-tool perspective, covering ALL the bugs the user hit in T15:
 *
 *   1. Generate keypair (WASM in-page) and complete the OAuth handshake
 *      programmatically (DCR → authorize-init → sign-challenge →
 *      authorize-callback → token). Mirrors what Cursor / VS Code /
 *      Claude.ai run when adding the connector.
 *
 *   2. POST /mcp `tools/call mnemonic_sign_memory` with Bearer JWT →
 *      server replies `awaiting_signature` with `approve_url` +
 *      `correlation_id` + `expires_in`. (Decision 12 deferred path —
 *      browser-mediated signing flow.)
 *
 *   3. Navigate to the approve_url path (`/sign/<correlation_id>`) in the
 *      same browser session that already has the keypair in
 *      localStorage. The page fetches `/api/pending/<id>` (capability
 *      auth — no JWT required), decodes the canonical-CBOR, renders the
 *      preview + countdown.
 *
 *   4. Click "Sign with my Mnemonic identity". The page calls
 *      WASM `sign_cose_payload(server_cbor_bytes, keypair)` and POSTs
 *      `{correlation_id, cose_signed_bytes, signer_pubkey}` to
 *      `/api/sign-callback`. Server validates the COSE chain (kid ↔
 *      jwt_sub ↔ stored content_hash) and persists the attestation.
 *
 *   5. POST /mcp `tools/call mnemonic_recall` with the SAME Bearer JWT →
 *      server returns the just-persisted attestation in the result hits.
 *      Cross-tool portability proven: a different MCP client with a
 *      JWT for the same identity sees the saved memory.
 *
 *   6. Negative — POST /mcp `tools/call mnemonic_recall` with a JWT for
 *      a DIFFERENT identity returns zero hits for the same query. Proves
 *      cross-tenant isolation (Decision 9, T15 spot-check #5b).
 */

test("deferred sign + cross-tool recall (browser + API in one harness)", async ({
  page,
  request,
}) => {
  test.setTimeout(120_000);

  // 1. OAuth handshake → JWT bound to a fresh keypair.
  const { pubkey: alicePubkey, jwt: aliceJwt } = await mintJwtViaOauth(
    page,
    request
  );

  // 2. tools/call sign_memory → awaiting_signature.
  const memoryContent = `e2e-test-memory ${randomHex(8)}`;
  const signMemoryResp = await request.post(`${mcpBase()}/mcp`, {
    headers: {
      Authorization: `Bearer ${aliceJwt}`,
      "Content-Type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 100,
      method: "tools/call",
      params: {
        name: "mnemonic_sign_memory",
        arguments: { content: memoryContent, tags: ["e2e", "playwright"] },
      },
    },
  });
  expect(signMemoryResp.status()).toBe(200);
  const signMemoryBody = await signMemoryResp.json();
  // MCP tools/call wraps the JSON return in `result.content[0].text`.
  const inner = JSON.parse(signMemoryBody.result.content[0].text);
  expect(inner.status).toBe("awaiting_signature");
  expect(inner.correlation_id).toMatch(/^[0-9a-f-]{36}$/);
  expect(inner.approve_url).toContain(`/sign/${inner.correlation_id}`);
  expect(inner.expires_in).toBeGreaterThanOrEqual(60);
  const correlationId = inner.correlation_id as string;

  // 3. Navigate to /sign/<id> — page must NOT bounce to /install
  //    (regression fix: cross-tool flow has keypair but no JWT in browser).
  await page.goto(`/sign/${correlationId}`);
  await expect(page).toHaveURL(new RegExp(`/sign/${correlationId}`));
  // Either the loading state or the ready state (countdown) is acceptable.
  await Promise.race([
    page.getByTestId("sign-countdown").waitFor({ timeout: 15_000 }),
    page.getByTestId("sign-error").waitFor({ timeout: 15_000 }),
    page.getByTestId("sign-expired").waitFor({ timeout: 15_000 }),
  ]);
  // Must reach the ready state (countdown visible).
  await expect(page.getByTestId("sign-countdown")).toBeVisible();

  // 4. Click "Sign" and wait for success banner with attestation_id.
  await page.getByTestId("sign-approve").click();
  await expect(page.getByTestId("sign-success")).toBeVisible({
    timeout: 30_000,
  });
  const attestationIdNode = page.getByTestId("sign-success-attestation-id");
  await expect(attestationIdNode).toBeVisible();
  const attestationLine = (await attestationIdNode.textContent())?.trim() ?? "";
  expect(attestationLine).toMatch(/attestation_id: [0-9a-f-]{36}/);

  // 5. Recall with Alice's JWT returns the just-persisted memory.
  const recallResp = await request.post(`${mcpBase()}/mcp`, {
    headers: {
      Authorization: `Bearer ${aliceJwt}`,
      "Content-Type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 101,
      method: "tools/call",
      params: {
        name: "mnemonic_recall",
        arguments: { query: memoryContent, limit: 5 },
      },
    },
  });
  expect(recallResp.status()).toBe(200);
  const recallBody = await recallResp.json();
  const recallText = recallBody.result.content[0].text as string;
  // The recall response includes the matched content verbatim — assert
  // our just-saved string is in there.
  expect(recallText).toContain(memoryContent);

  // 6. Cross-tenant isolation — Bob recalls with Alice's content,
  //    server returns zero hits.
  // Use a fresh page context to keep Bob's keypair separate.
  const bobContext = await page.context().browser()!.newContext();
  const bobPage = await bobContext.newPage();
  const { jwt: bobJwt, pubkey: bobPubkey } = await mintJwtViaOauth(
    bobPage,
    request
  );
  expect(bobPubkey).not.toBe(alicePubkey);
  await bobContext.close();

  const bobRecallResp = await request.post(`${mcpBase()}/mcp`, {
    headers: {
      Authorization: `Bearer ${bobJwt}`,
      "Content-Type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 102,
      method: "tools/call",
      params: {
        name: "mnemonic_recall",
        arguments: { query: memoryContent, limit: 5 },
      },
    },
  });
  expect(bobRecallResp.status()).toBe(200);
  const bobRecallBody = await bobRecallResp.json();
  const bobText = bobRecallBody.result.content[0].text as string;
  // Bob must NOT see Alice's content. The server emits "0 results" or
  // similar empty-set framing when no rows match the owner-scoped query.
  expect(bobText).not.toContain(memoryContent);
});

test("/sign/<id> with no keypair redirects to /install", async ({ page }) => {
  // Negative: a fresh browser visiting /sign/<id> directly (no /install
  // detour first) has no keypair → redirect to /install. Regression
  // guard for the cross-tool fix that *kept* the redirect for the
  // genuinely-no-identity case.
  await page.goto("/install"); // load WASM-capable page first
  await page.evaluate(() => {
    localStorage.removeItem("mnemonic.identity");
    localStorage.removeItem("mnemonic.jwt");
  });
  await page.goto("/sign/00000000-0000-0000-0000-000000000000");
  await expect(page).toHaveURL(/\/install$/, { timeout: 5_000 });
});

test("sign-callback replay returns 410 Gone (single-use)", async ({
  page,
  request,
}) => {
  test.setTimeout(60_000);
  const { pubkey, jwt } = await mintJwtViaOauth(page, request);

  // Stage a pending bundle.
  const content = `replay-test ${randomHex(6)}`;
  const r = await request.post(`${mcpBase()}/mcp`, {
    headers: {
      Authorization: `Bearer ${jwt}`,
      "Content-Type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "mnemonic_sign_memory", arguments: { content } },
    },
  });
  const inner = JSON.parse((await r.json()).result.content[0].text);
  const correlationId = inner.correlation_id as string;

  // Sign once via the UI.
  await page.goto(`/sign/${correlationId}`);
  await page.getByTestId("sign-approve").click();
  await expect(page.getByTestId("sign-success")).toBeVisible({
    timeout: 30_000,
  });

  // Re-fetch the canonical-CBOR — should now 410 (consumed).
  const peekResp = await request.get(
    `${mcpBase()}/api/pending/${correlationId}`
  );
  expect([410, 404]).toContain(peekResp.status());

  // Direct callback replay attempt: forge a payload — the server must reject
  // because the bundle is already consumed. We don't bother re-signing
  // (any garbage is fine; consumed → 410 short-circuit before COSE verify).
  const replay = await request.post(`${mcpBase()}/api/sign-callback`, {
    data: {
      correlation_id: correlationId,
      cose_signed_bytes: "AAAA",
      signer_pubkey: pubkey,
    },
  });
  expect(replay.status()).toBe(410);
});

/**
 * End-to-end OAuth handshake helper that:
 *   - generates a keypair via the IdentityPanel UI (so WASM is loaded in
 *     the page context and the localStorage shape matches production)
 *   - runs the full DCR + authorize + token flow in JSON mode against
 *     the configured MCP backend
 *
 * Returns the resulting Bearer JWT and the user's base58 pubkey.
 *
 * Side effect: leaves the page on `/install` with the keypair stored in
 * localStorage. Subsequent navigations in the SAME page share the same
 * identity (e.g. `/sign/<id>` reads `mnemonic.identity`).
 */
async function mintJwtViaOauth(
  page: Page,
  request: import("@playwright/test").APIRequestContext
): Promise<{ pubkey: string; jwt: string }> {
  // Generate keypair via the UI (loads WASM, populates localStorage).
  await page.goto("/install");
  await page.getByTestId("identity-generate").click();
  await expect(page.getByTestId("identity-pubkey")).toBeVisible({
    timeout: 10_000,
  });
  const pubkey = (
    await page.getByTestId("identity-pubkey").textContent()
  )?.trim() as string;

  // RFC 7591 DCR.
  const dcrResp = await request.post(`${mcpBase()}/oauth/register`, {
    data: {
      client_name: "playwright-e2e",
      redirect_uris: ["http://127.0.0.1:9999/cb"],
    },
  });
  const { client_id: clientId } = (await dcrResp.json()) as {
    client_id: string;
  };

  // /oauth/authorize JSON mode (pubkey hint + Accept header).
  const codeVerifier = randomHex(32);
  const codeChallenge = await pkceChallenge(codeVerifier);
  const state = randomHex(16);
  const initResp = await request.get(`${mcpBase()}/oauth/authorize`, {
    params: {
      response_type: "code",
      client_id: clientId,
      redirect_uri: "http://127.0.0.1:9999/cb",
      code_challenge: codeChallenge,
      code_challenge_method: "S256",
      state,
      pubkey,
    },
    headers: { Accept: "application/json" },
  });
  const init = (await initResp.json()) as {
    challenge_cbor: string;
    state: string;
    exp: number;
  };

  // WASM sign_challenge inside the page (keypair lives in localStorage).
  // Indirect-import via the Function constructor so TypeScript does NOT
  // try to resolve the Vite dev-server URL `/src/lib/wasm.ts` statically.
  const sigB64 = await page.evaluate(async (b64: string) => {
    const dynamicImport = new Function("p", "return import(p)") as (
      p: string
    ) => Promise<unknown>;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const mod = (await dynamicImport("/src/lib/wasm.ts")) as any;
    const wasm = await mod.loadWasm();
    const identity = JSON.parse(
      localStorage.getItem("mnemonic.identity") as string
    );
    const challengeBytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const sig = wasm.sign_challenge(identity, challengeBytes);
    let bin = "";
    for (let i = 0; i < sig.length; i++) bin += String.fromCharCode(sig[i]);
    return btoa(bin);
  }, init.challenge_cbor);

  // POST /oauth/authorize → code.
  const authResp = await request.post(`${mcpBase()}/oauth/authorize`, {
    data: { state, signature: sigB64, signer_pubkey: pubkey },
  });
  const auth = (await authResp.json()) as {
    code: string;
    state: string;
    redirect_uri: string;
  };

  // POST /oauth/token (form-urlencoded — VS Code default).
  const tokenResp = await request.post(`${mcpBase()}/oauth/token`, {
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    data: new URLSearchParams({
      grant_type: "authorization_code",
      code: auth.code,
      code_verifier: codeVerifier,
      redirect_uri: "http://127.0.0.1:9999/cb",
      client_id: clientId,
    }).toString(),
  });
  const token = (await tokenResp.json()) as { access_token: string };
  return { pubkey, jwt: token.access_token };
}
