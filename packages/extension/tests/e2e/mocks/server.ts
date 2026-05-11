// T19 — `mc.mnemonik.xyz` server-side mock for Playwright extension
// tests. Routes are installed at the context level so every page (popup,
// options, content scripts, service worker fetch) picks them up.
//
// Each test customizes the fixture object passed to `mockServer()`.
// The handler reads `state` from a small map so a single context can
// flip a route's response mid-test (e.g. wrong-passphrase rate-limit:
// 200 → 200 → 200 → 200 → 200 → 429).

import type { BrowserContext, Route } from "@playwright/test";
import { Buffer } from "node:buffer";

export interface ServerMockState {
  /** Google sub → look-up response (existing pubkey + escrow flag). */
  lookupByGoogleSub: Record<
    string,
    { pubkey_base58: string | null; escrow_present: boolean }
  >;
  /** Escrow blob keyed by Google sub. */
  escrowByGoogleSub: Record<string, EscrowBlobLike | undefined>;
  /** Force GET /api/key-escrow to 429 after this many calls. */
  escrowGetRateLimitAfter?: number;
  /** Retry-After (seconds) returned alongside the 429. */
  escrowRetryAfterSeconds: number;
  /** Outgoing extension JWT (returned by /api/extension-bootstrap/redeem). */
  extensionJwt: string;
  /** Outgoing mcp JWT (returned by /oauth/token, google_sub embedded). */
  mcpJwtFor(googleSub: string): string;
  /** sign-callback returns these tx ids. */
  attestationId: string;
  solanaTx: string;
  arweaveTx: string;
}

export interface EscrowBlobLike {
  ciphertext_base64: string;
  nonce_base64: string;
  salt_base64: string;
  pubkey_base58: string;
  argon2_params: { m_cost: number; t_cost: number; p_cost: number };
}

export function defaultMockState(): ServerMockState {
  return {
    lookupByGoogleSub: {},
    escrowByGoogleSub: {},
    escrowRetryAfterSeconds: 30,
    extensionJwt: "ext.jwt.fake",
    mcpJwtFor: (sub) =>
      "mcp." +
      Buffer.from(JSON.stringify({ sub, aud: "mcp" })).toString("base64url") +
      ".sig",
    attestationId: "att-fake-1",
    solanaTx: "solana:fake:1",
    arweaveTx: "arweave:fake:1",
  };
}

export interface ServerMockHandle {
  state: ServerMockState;
  counts: Record<string, number>;
  dispose: () => Promise<void>;
}

/** Install all server-side route handlers. The returned `state` object
 *  is live — tests mutate it between requests to switch responses. */
export async function mockServer(
  context: BrowserContext,
  initial: Partial<ServerMockState> = {},
): Promise<ServerMockHandle> {
  const state: ServerMockState = { ...defaultMockState(), ...initial };
  const counts: Record<string, number> = {};
  const hit = (k: string): number => (counts[k] = (counts[k] ?? 0) + 1);

  const json = (
    route: Route,
    status: number,
    body: unknown,
    headers: Record<string, string> = {},
  ) =>
    route.fulfill({
      status,
      headers: { "content-type": "application/json", ...headers },
      body: JSON.stringify(body),
    });

  await context.route(/https:\/\/mc\.mnemonik\.xyz\/.*/, async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname;
    hit(`${req.method()} ${path}`);

    // /oauth/token — exchange auth_code for an aud=mcp JWT.
    if (path === "/oauth/token" && req.method() === "POST") {
      const body = safeJson(req.postData()) ?? {};
      const sub =
        (body as Record<string, string>).google_sub ?? "test-google-sub-001";
      return json(route, 200, {
        access_token: state.mcpJwtFor(sub),
        token_type: "Bearer",
        expires_in: 3600,
        google_sub: sub,
        email: (body as Record<string, string>).email ?? "tester@example.com",
      });
    }

    // /api/extension-bootstrap/issue → ticket
    if (path === "/api/extension-bootstrap/issue" && req.method() === "POST") {
      return json(route, 200, {
        ticket: "ticket-" + (counts[`POST ${path}`] ?? 0),
        ttl_seconds: 30,
      });
    }
    // /api/extension-bootstrap/redeem/:ticket → ext JWT
    if (
      path.startsWith("/api/extension-bootstrap/redeem/") &&
      req.method() === "GET"
    ) {
      return json(route, 200, {
        access_token: state.extensionJwt,
        expires_in: 600,
      });
    }

    // /oauth/google/lookup → pubkey + escrow_present
    if (path === "/oauth/google/lookup" && req.method() === "POST") {
      const body = safeJson(req.postData()) ?? {};
      const sub = (body as Record<string, string>).google_sub;
      const entry = sub ? state.lookupByGoogleSub[sub] : undefined;
      return json(
        route,
        200,
        entry ?? { pubkey_base58: null, escrow_present: false },
      );
    }
    // /oauth/google/link
    if (path === "/oauth/google/link" && req.method() === "POST") {
      return json(route, 200, { ok: true });
    }

    // /api/key-escrow — PUT / GET / DELETE
    if (path === "/api/key-escrow") {
      const sub =
        parseJwtSub(req.headers()["authorization"]) ?? "test-google-sub-001";
      if (req.method() === "PUT") {
        const body = safeJson(req.postData()) as EscrowBlobLike | undefined;
        if (body) state.escrowByGoogleSub[sub] = body;
        return json(route, 200, { ok: true });
      }
      if (req.method() === "GET") {
        const calls = counts[`GET ${path}`] ?? 0;
        if (
          typeof state.escrowGetRateLimitAfter === "number" &&
          calls > state.escrowGetRateLimitAfter
        ) {
          return json(
            route,
            429,
            { error: "rate_limited" },
            { "retry-after": String(state.escrowRetryAfterSeconds) },
          );
        }
        const blob = state.escrowByGoogleSub[sub];
        if (!blob) return json(route, 404, { error: "not_found" });
        return json(route, 200, blob);
      }
      if (req.method() === "DELETE") {
        delete state.escrowByGoogleSub[sub];
        return json(route, 200, { ok: true });
      }
    }

    // /mcp — mnemonic_sign_memory returns a correlation id (deferred-sign).
    if (path === "/mcp" && req.method() === "POST") {
      return json(route, 200, {
        jsonrpc: "2.0",
        id: 1,
        result: {
          correlation_id: "corr-" + (counts[`POST /mcp`] ?? 0),
          status: "pending",
        },
      });
    }

    // /api/sign-callback → attestation id + tx ids
    if (path === "/api/sign-callback" && req.method() === "POST") {
      return json(route, 200, {
        attestation_id: state.attestationId,
        solana_tx: state.solanaTx,
        arweave_tx: state.arweaveTx,
      });
    }

    // Unknown route — fall through to a 404 so tests fail loudly.
    return json(route, 404, { error: "no_mock_for", path });
  });

  const dispose = async () => {
    await context.unroute(/https:\/\/mc\.mnemonik\.xyz\/.*/);
  };
  return { state, counts, dispose };
}

function safeJson(body: string | null): unknown {
  if (!body) return undefined;
  try {
    return JSON.parse(body);
  } catch {
    return undefined;
  }
}

/** Extract the `sub` claim from a Bearer token in the Authorization header.
 *  The harness mints JWT-shaped strings whose middle segment is a base64url
 *  JSON blob — we don't bother verifying the signature in tests. */
function parseJwtSub(authz: string | undefined): string | null {
  if (!authz) return null;
  const m = /^Bearer\s+(\S+)$/.exec(authz);
  if (!m) return null;
  const parts = m[1]!.split(".");
  if (parts.length < 2) return null;
  try {
    const json = Buffer.from(parts[1]!, "base64url").toString("utf8");
    const parsed = JSON.parse(json) as { sub?: string };
    return parsed.sub ?? null;
  } catch {
    return null;
  }
}
