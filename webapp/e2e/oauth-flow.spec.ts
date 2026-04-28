import { test, expect, type Page } from "@playwright/test";
import {
  mcpBase,
  base64url,
  base64Std,
  randomHex,
  pkceChallenge,
} from "./_helpers";

// Serial + no retries — /oauth/* is rate-limited at 5 req/min/IP by
// tower_governor (Decision 9). Each OAuth handshake makes 4 /oauth/*
// calls; with default 2 retries on failure that's 12 calls/test = blows
// the cap immediately. Disable retries: a real failure is fail, period.
test.describe.configure({ mode: "serial", retries: 0 });

/**
 * Browser-driven OAuth 2.1 + PKCE happy path against the live (or local)
 * mnemonic-mcp backend.
 *
 * Drives the full RFC 6749 §4.1 + RFC 7636 dance:
 *
 *   1. POST /oauth/register (RFC 7591 DCR) — opaque client_id.
 *   2. WASM `generate_keypair` in the page; reads pubkey from localStorage.
 *   3. GET  /oauth/authorize?...&pubkey=<base58>  with  Accept: application/json
 *      → returns `{challenge_cbor, state, exp}` instead of redirecting to
 *      the consent page (the `pubkey` hint + Accept header switch the
 *      handler to JSON mode — see `oauth.rs::authorize_init_handler`).
 *   4. WASM `sign_challenge(keypair, challenge_bytes)` in-page → 64-byte
 *      Ed25519 signature, base64.
 *   5. POST /oauth/authorize with `{state, signature, signer_pubkey}`
 *      → `{code, state, redirect_uri}`.
 *   6. POST /oauth/token form-urlencoded with `code + code_verifier` →
 *      `{access_token, token_type:"Bearer", expires_in, scope:"mcp"}`.
 *   7. POST /mcp tools/call with Bearer JWT → guard that the issued
 *      token actually authorizes a tools/call (the OAuth round-trip
 *      doesn't *prove* the token is usable until we exchange it for a
 *      `tools/call` response).
 *
 * This is the same handshake VS Code / Claude.ai run; if it stays green
 * the connector install flow stays unblocked.
 */

test("full OAuth handshake yields a working JWT", async ({ page, request }) => {
  test.setTimeout(60_000);

  // 1. DCR — client_name + redirect_uris are echoed back per RFC 7591.
  const dcrResp = await request.post(`${mcpBase()}/oauth/register`, {
    data: {
      client_name: "playwright-e2e",
      redirect_uris: ["http://127.0.0.1:9999/cb"],
      grant_types: ["authorization_code"],
      response_types: ["code"],
      token_endpoint_auth_method: "none",
    },
  });
  expect(dcrResp.status()).toBe(201);
  const dcr = await dcrResp.json();
  expect(dcr.client_id).toMatch(/^[0-9a-f-]{36}$/);
  const clientId = dcr.client_id as string;

  // 2. WASM-generated keypair via the IdentityPanel UI.
  await page.goto("/install");
  await page.getByTestId("identity-generate").click();
  await expect(page.getByTestId("identity-pubkey")).toBeVisible({
    timeout: 10_000,
  });
  const pubkey = (
    await page.getByTestId("identity-pubkey").textContent()
  )?.trim() as string;
  expect(pubkey).toBeTruthy();

  // 3. GET /oauth/authorize in JSON mode (pubkey hint + Accept header).
  const codeVerifier = randomHex(32);
  const codeChallenge = await pkceChallenge(codeVerifier);
  const state = randomHex(16);
  const redirectUri = "http://127.0.0.1:9999/cb";
  const initResp = await request.get(`${mcpBase()}/oauth/authorize`, {
    params: {
      response_type: "code",
      client_id: clientId,
      redirect_uri: redirectUri,
      code_challenge: codeChallenge,
      code_challenge_method: "S256",
      state,
      pubkey,
    },
    headers: { Accept: "application/json" },
  });
  expect(initResp.status()).toBe(200);
  const init = await initResp.json();
  expect(init.state).toBe(state);
  expect(typeof init.challenge_cbor).toBe("string");

  // 4. Sign the challenge bytes inside the page (WASM is loaded post-/install).
  const sigB64 = await signChallengeInPage(page, init.challenge_cbor);

  // 5. POST /oauth/authorize → code.
  const authResp = await request.post(`${mcpBase()}/oauth/authorize`, {
    data: { state, signature: sigB64, signer_pubkey: pubkey },
  });
  expect(authResp.status()).toBe(200);
  const auth = await authResp.json();
  expect(auth.state).toBe(state);
  expect(auth.redirect_uri).toBe(redirectUri);
  expect(typeof auth.code).toBe("string");

  // 6. POST /oauth/token (form-urlencoded — VS Code / Claude.ai default).
  const tokenResp = await request.post(`${mcpBase()}/oauth/token`, {
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    data: new URLSearchParams({
      grant_type: "authorization_code",
      code: auth.code,
      code_verifier: codeVerifier,
      redirect_uri: redirectUri,
      client_id: clientId,
    }).toString(),
  });
  expect(tokenResp.status()).toBe(200);
  const token = await tokenResp.json();
  expect(token.token_type).toBe("Bearer");
  expect(token.scope).toBe("mcp");
  expect(typeof token.access_token).toBe("string");

  // 7. The JWT actually authorizes a tools/call.
  const callResp = await request.post(`${mcpBase()}/mcp`, {
    headers: {
      Authorization: `Bearer ${token.access_token}`,
      "Content-Type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "mnemonic_recall",
        arguments: { query: "non-existent-query-for-auth-smoke" },
      },
    },
  });
  expect(callResp.status()).toBe(200); // 200 even on empty result.
  const callBody = await callResp.json();
  expect(callBody.jsonrpc).toBe("2.0");
});

test("alg=none JWT submission is rejected", async ({ request }) => {
  // Defense in depth — Decision 11 fixes the verifier algorithm at HS256.
  // A token with `alg: none` (forged header) must NOT pass.
  const algNoneJwt = makeAlgNoneJwt({
    sub: "fake-pubkey-12345",
    iss: mcpBase().replace(/^https?:\/\//, ""),
    aud: "mcp",
  });
  const resp = await request.post(`${mcpBase()}/mcp`, {
    headers: {
      Authorization: `Bearer ${algNoneJwt}`,
      "Content-Type": "application/json",
    },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "mnemonic_recall",
        arguments: { query: "x" },
      },
    },
  });
  expect(resp.status()).toBe(401);
});

/**
 * Calls `wasm.sign_challenge(identity, challenge_bytes)` inside the page
 * context. The page must have completed `/install` already so localStorage
 * holds `mnemonic.identity` and `loadWasm()` resolves.
 */
async function signChallengeInPage(
  page: Page,
  challengeB64: string
): Promise<string> {
  return await page.evaluate(async (b64) => {
    // The wasm loader caches its init promise — first call here primes
    // it; subsequent calls reuse the same module. Indirect-import via the
    // Function constructor so TypeScript does NOT try to resolve the Vite
    // dev-server URL `/src/lib/wasm.ts` at compile time.
    const dynamicImport = new Function("p", "return import(p)") as (
      p: string
    ) => Promise<unknown>;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const mod = (await dynamicImport("/src/lib/wasm.ts")) as any;
    const wasm = await mod.loadWasm();
    const identityRaw = localStorage.getItem("mnemonic.identity");
    if (!identityRaw) throw new Error("no identity in localStorage");
    const identity = JSON.parse(identityRaw);
    const challengeBytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const sig = wasm.sign_challenge(identity, challengeBytes);
    let bin = "";
    for (let i = 0; i < sig.length; i++) bin += String.fromCharCode(sig[i]);
    return btoa(bin);
  }, challengeB64);
}

/**
 * Build an unsigned `alg: none` JWT for the alg-confusion negative test.
 * This is intentionally a forgery — the server must reject it.
 */
function makeAlgNoneJwt(claims: Record<string, unknown>): string {
  const now = Math.floor(Date.now() / 1000);
  const header = base64url(
    new TextEncoder().encode(JSON.stringify({ alg: "none", typ: "JWT" }))
  );
  const payload = base64url(
    new TextEncoder().encode(
      JSON.stringify({
        iat: now,
        exp: now + 600,
        jti: "alg-none-attack",
        ...claims,
      })
    )
  );
  // No signature segment → `alg: none` is the literal bypass attempt.
  return `${header}.${payload}.`;
}

// Re-export base64Std so other specs can pull it from here without crossing
// _helpers.ts to a different relative path.
export { base64Std };
