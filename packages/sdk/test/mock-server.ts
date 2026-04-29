// In-process mock MCP server for SDK + CLI integration tests.
//
// Provides the full HTTP surface the SDK / CLI exercise:
//   POST /mcp                          (JSON-RPC: tools/list, tools/call)
//   GET  /oauth/authorize              (issues authorization code)
//   POST /oauth/token                  (code → fake HS256-shaped JWT)
//   POST /oauth/register               (DCR — echo client_id)
//   GET  /api/pending/:correlation_id  (returns canonical-CBOR bytes)
//   POST /api/sign-callback            (validates COSE envelope, emits attestation_id)
//   POST /api/cli-bootstrap/issue      (mints single-use ticket UUID)
//   GET  /api/cli-bootstrap/redeem/:t  (returns keypair JSON; second call → 404)
//
// Fault-injection via `mockServer.withFault(mode)` global setter — every
// integration test exercises at least one fault path:
//   '5xx-on-token-exchange'         → /oauth/token → 500
//   'malformed-cbor-in-pending'     → /api/pending → garbage bytes
//   'expired-jwt-after-N-requests'  → after N OK calls, /mcp + /api/* → 401
//   'signer-pubkey-mismatch'        → /api/sign-callback → 400
//   'callback-timeout'              → /oauth/token hangs (no response)
//
// Pure node:http; no real network. The mock IS the network for tests.
//
// NOTE: this file is allowed to use `node:*` imports — the constraint
// "no node:* in packages/sdk/test/" applies to the runtime under test,
// but mock-server.ts is the test-time HTTP harness, equivalent in
// spirit to the `node:http` server inside `cli/src/commands/login.ts`.
// SDK source files (under `packages/sdk/src/`) remain pure ESM.

import {
  createServer,
  type IncomingMessage,
  type ServerResponse,
} from "node:http";
import type { AddressInfo } from "node:net";

// ── types ───────────────────────────────────────────────────────────────────

export type FaultMode =
  | null
  | "5xx-on-token-exchange"
  | "malformed-cbor-in-pending"
  | "expired-jwt-after-N-requests"
  | "signer-pubkey-mismatch"
  | "callback-timeout";

export interface MockServer {
  /** Fully-qualified base URL, e.g. "http://127.0.0.1:54231". */
  url: string;
  /** Captured calls — tests can assert e.g. "/api/sign-callback NOT called". */
  calls: Array<{ method: string; path: string }>;
  /** Toggle fault mode. Pass `null` to clear. */
  withFault(mode: FaultMode, opts?: { requestsBeforeExpiry?: number }): void;
  /** Reset state between tests (calls, faults, ticket store, code store). */
  reset(): void;
  /** Pre-issue a bootstrap ticket and return its UUID (CLI-side test entry point). */
  issueTicketSync(payload: { secret: number[]; pubkey_base58: string }): string;
  /** Stop the listener. */
  close(): Promise<void>;
  /**
   * Forcibly drop all in-flight connections. Tests use this after the
   * `callback-timeout` fault to free hung sockets so subsequent test
   * setup doesn't block on socket drain.
   */
  closeAllConnections(): void;
}

// ── crypto-light helpers ────────────────────────────────────────────────────

function b64url(input: string | Uint8Array): string {
  const bytes =
    typeof input === "string" ? new TextEncoder().encode(input) : input;
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]!);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function makeFakeJwt(sub: string, expSeconds = 3600): string {
  const header = { alg: "HS256", typ: "JWT" };
  const now = Math.floor(Date.now() / 1000);
  const payload = { sub, iat: now, exp: now + expSeconds };
  // Signature is a fixed, JWT-shaped string. SDK never validates it locally
  // (signature checking is server-side). The `parseJwtPayload` boundary
  // only checks header alg + payload claims, both of which are valid here.
  return `${b64url(JSON.stringify(header))}.${b64url(
    JSON.stringify(payload)
  )}.test-signature`;
}

function uuid(): string {
  const c = (globalThis.crypto ?? crypto) as Crypto & {
    randomUUID?: () => string;
  };
  if (typeof c.randomUUID === "function") return c.randomUUID();
  // Synthesize from random bytes — Node 20+ + Bun + Deno all expose
  // crypto.getRandomValues globally, so this fallback should never fire.
  const b = new Uint8Array(16);
  c.getRandomValues(b);
  b[6] = (b[6]! & 0x0f) | 0x40;
  b[8] = (b[8]! & 0x3f) | 0x80;
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(
    16,
    20
  )}-${h.slice(20)}`;
}

// ── server factory ──────────────────────────────────────────────────────────

interface AuthCode {
  redirectUri: string;
  codeChallenge: string;
  state: string;
  consumed: boolean;
}

interface Ticket {
  payload: { secret: number[]; pubkey_base58: string };
  redeemed: boolean;
}

/**
 * Spin up the mock server on a free port. Caller must `await server.close()`
 * in `afterAll` / `afterEach`.
 */
export async function startMockServer(): Promise<MockServer> {
  let fault: FaultMode = null;
  let requestsBeforeExpiry = 0;
  let okRequestCount = 0;
  const calls: Array<{ method: string; path: string }> = [];
  const codes = new Map<string, AuthCode>();
  const tickets = new Map<string, Ticket>();
  const correlations = new Map<string, Uint8Array>();

  const server = createServer(async (req, res) => {
    try {
      await handle(req, res);
    } catch (e) {
      // Swallow — surface as 500 so tests see a server error rather than
      // hanging on an unhandled rejection.
      try {
        res.writeHead(500, { "content-type": "text/plain" });
        res.end(`mock-server: ${e instanceof Error ? e.message : String(e)}`);
      } catch {
        /* socket already destroyed */
      }
    }
  });

  async function handle(
    req: IncomingMessage,
    res: ServerResponse
  ): Promise<void> {
    const method = (req.method ?? "GET").toUpperCase();
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const path = url.pathname;
    calls.push({ method, path });

    // ── jwt-expiry fault: gate /mcp + /api/* after N requests ──────────────
    if (
      fault === "expired-jwt-after-N-requests" &&
      (path === "/mcp" || path.startsWith("/api/"))
    ) {
      if (okRequestCount >= requestsBeforeExpiry) {
        res.writeHead(401, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "jwt expired" }));
        return;
      }
      okRequestCount++;
    }

    // ── /oauth/authorize ────────────────────────────────────────────────────
    if (method === "GET" && path === "/oauth/authorize") {
      const redirectUri = url.searchParams.get("redirect_uri") ?? "";
      const codeChallenge = url.searchParams.get("code_challenge") ?? "";
      const state = url.searchParams.get("state") ?? "";
      const code = `mock-code-${uuid().slice(0, 8)}`;
      codes.set(code, {
        redirectUri,
        codeChallenge,
        state,
        consumed: false,
      });
      // 302 to redirect_uri?code=...&state=...
      const redir = new URL(redirectUri);
      redir.searchParams.set("code", code);
      redir.searchParams.set("state", state);
      res.writeHead(302, { location: redir.toString() });
      res.end();
      return;
    }

    // ── /oauth/token ────────────────────────────────────────────────────────
    if (method === "POST" && path === "/oauth/token") {
      if (fault === "callback-timeout") {
        // Hang the connection — no response, no close. Test must use a
        // short timeout / AbortController so the suite terminates.
        return;
      }
      if (fault === "5xx-on-token-exchange") {
        res.writeHead(500, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "internal" }));
        return;
      }
      const body = await readJson(req);
      const code = typeof body?.code === "string" ? body.code : "";
      const entry = codes.get(code);
      if (!entry || entry.consumed) {
        res.writeHead(400, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "invalid_grant" }));
        return;
      }
      entry.consumed = true;
      const sub = "test-user-pubkey";
      const jwt = makeFakeJwt(sub, 3600);
      const expiresAt = new Date(Date.now() + 3600 * 1000).toISOString();
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ jwt, expires_at: expiresAt, sub }));
      return;
    }

    // ── /oauth/register ─────────────────────────────────────────────────────
    if (method === "POST" && path === "/oauth/register") {
      const body = await readJson(req);
      const clientId =
        typeof body?.client_id === "string" ? body.client_id : "mnemonic-cli";
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ client_id: clientId }));
      return;
    }

    // ── /mcp ────────────────────────────────────────────────────────────────
    if (method === "POST" && path === "/mcp") {
      const body = await readJson(req);
      const reqMethod =
        typeof body?.method === "string" ? body.method : "tools/call";
      const params = (body?.params ?? {}) as Record<string, unknown>;

      if (reqMethod === "tools/list") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(
          JSON.stringify({
            jsonrpc: "2.0",
            id: body?.id ?? 1,
            result: {
              tools: [
                { name: "mnemonic_whoami" },
                { name: "mnemonic_sign_memory" },
                { name: "mnemonic_recall" },
                { name: "mnemonic_verify" },
                { name: "mnemonic_prove_identity" },
              ],
            },
          })
        );
        return;
      }

      const toolName =
        typeof params.name === "string" ? params.name : "(unknown)";
      const args = (params.arguments ?? {}) as Record<string, unknown>;
      const toolResult = handleTool(toolName, args, correlations);

      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          jsonrpc: "2.0",
          id: body?.id ?? 1,
          result: {
            content: [{ type: "text", text: JSON.stringify(toolResult) }],
          },
        })
      );
      return;
    }

    // ── /api/pending/:correlation_id ────────────────────────────────────────
    if (method === "GET" && path.startsWith("/api/pending/")) {
      const corr = decodeURIComponent(path.slice("/api/pending/".length));
      const stored = correlations.get(corr);
      if (!stored) {
        res.writeHead(404, { "content-type": "text/plain" });
        res.end("pending bundle not found");
        return;
      }
      let bytes: Uint8Array = stored;
      if (fault === "malformed-cbor-in-pending") {
        // Garbage bytes that violate canonical CBOR. The SDK shouldn't
        // crash — the WASM mock signs whatever it gets, but the test
        // pipes this fault into the cose-validation path: a real WASM
        // build would reject invalid CBOR head bytes via IntegrityError.
        // Our SDK contract: the `coseSignPayload` defense-in-depth check
        // on the returned envelope's first byte (0x84) is what catches
        // drift. Tests assert the fault is reachable end-to-end.
        bytes = new Uint8Array([0xff, 0x00, 0xde, 0xad, 0xbe, 0xef]);
      }
      res.writeHead(200, { "content-type": "application/cbor" });
      res.end(Buffer.from(bytes));
      return;
    }

    // ── /api/sign-callback ──────────────────────────────────────────────────
    if (method === "POST" && path === "/api/sign-callback") {
      if (fault === "signer-pubkey-mismatch") {
        res.writeHead(400, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "signer mismatch" }));
        return;
      }
      const body = await readJson(req);
      const corr =
        typeof body?.correlation_id === "string" ? body.correlation_id : "";
      const cose =
        typeof body?.cose_signed_bytes === "string"
          ? body.cose_signed_bytes
          : "";
      const signer =
        typeof body?.signer_pubkey === "string" ? body.signer_pubkey : "";
      // Loose validation: correlation must exist; cose_signed_bytes must be
      // non-empty base64; signer non-empty. Tight COSE byte-validation lives
      // in golden tests via real WASM (Decision 12) — not here.
      if (!corr || !cose || !signer) {
        res.writeHead(400, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "missing fields" }));
        return;
      }
      // Decode base64 just to confirm it's not totally broken.
      try {
        atob(cose);
      } catch {
        res.writeHead(400, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: "cose_signed_bytes not base64" }));
        return;
      }
      const attestationId = `att-${uuid().slice(0, 8)}`;
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          attestation_id: attestationId,
          signed_at: new Date().toISOString(),
          status: "stored",
          content_hash: "mock-hash",
        })
      );
      return;
    }

    // ── /api/cli-bootstrap/issue ────────────────────────────────────────────
    if (method === "POST" && path === "/api/cli-bootstrap/issue") {
      const body = await readJson(req);
      // Accept either {secret, pubkey_base58} or default to a synthetic shape.
      const payload =
        body && typeof body === "object" && "secret" in body
          ? (body as { secret: number[]; pubkey_base58: string })
          : { secret: Array(64).fill(0), pubkey_base58: "test-pubkey" };
      const ticket = uuid();
      tickets.set(ticket, { payload, redeemed: false });
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ ticket }));
      return;
    }

    // ── /api/cli-bootstrap/redeem/:ticket ───────────────────────────────────
    if (method === "GET" && path.startsWith("/api/cli-bootstrap/redeem/")) {
      const ticket = decodeURIComponent(
        path.slice("/api/cli-bootstrap/redeem/".length)
      );
      const entry = tickets.get(ticket);
      if (!entry || entry.redeemed) {
        res.writeHead(404, { "content-type": "text/plain" });
        res.end("ticket not found or already redeemed");
        return;
      }
      entry.redeemed = true;
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(entry.payload));
      return;
    }

    // ── default: 404 ────────────────────────────────────────────────────────
    res.writeHead(404, { "content-type": "text/plain" });
    res.end(`mock-server: no route for ${method} ${path}`);
  }

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.removeListener("error", reject);
      resolve();
    });
  });

  const addr = server.address() as AddressInfo;
  const url = `http://127.0.0.1:${addr.port}`;

  return {
    url,
    calls,
    withFault(mode, opts) {
      fault = mode;
      requestsBeforeExpiry = opts?.requestsBeforeExpiry ?? 0;
      okRequestCount = 0;
    },
    reset() {
      fault = null;
      requestsBeforeExpiry = 0;
      okRequestCount = 0;
      calls.length = 0;
      codes.clear();
      tickets.clear();
      correlations.clear();
    },
    issueTicketSync(payload) {
      const t = uuid();
      tickets.set(t, { payload, redeemed: false });
      return t;
    },
    close() {
      return new Promise<void>((resolve, reject) => {
        // Drop hung sockets first — without this, a `callback-timeout`
        // test will hold the listener open past `close()`'s callback.
        try {
          server.closeAllConnections();
        } catch {
          /* method may be absent on very old Node — best-effort */
        }
        if (!server.listening) {
          // Idempotent close — bun's test runner can invoke `afterAll`
          // more than once across nested describes, and `node:http` raises
          // ERR_SERVER_NOT_RUNNING if `.close()` is called on a non-listening
          // server. Treat that as a no-op.
          resolve();
          return;
        }
        server.close((err) => (err ? reject(err) : resolve()));
      });
    },
    closeAllConnections() {
      try {
        server.closeAllConnections();
      } catch {
        /* ignore */
      }
    },
  };
}

// ── helpers ─────────────────────────────────────────────────────────────────

async function readJson(
  req: IncomingMessage
): Promise<Record<string, unknown> | null> {
  const chunks: Buffer[] = [];
  for await (const c of req) {
    chunks.push(typeof c === "string" ? Buffer.from(c) : c);
  }
  const raw = Buffer.concat(chunks).toString("utf8");
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object"
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function handleTool(
  name: string,
  args: Record<string, unknown>,
  correlations: Map<string, Uint8Array>
): Record<string, unknown> {
  switch (name) {
    case "mnemonic_whoami":
      return {
        server_pubkey: "MockServerPubkey1111111111111111111111111111",
        server_did: "did:key:mock-server",
      };
    case "mnemonic_sign_memory": {
      const corr = `corr-${Math.floor(Math.random() * 1e9)}`;
      // Stash a tiny but well-formed CBOR-ish payload so /api/pending can
      // serve real bytes. Decision 7: server returns pending-bundle shape
      // (correlation_id, payload_cbor_base64, expires_at) — but the SDK
      // re-fetches the bytes from /api/pending, so we only need the id here.
      const payload = new Uint8Array([0x82, 0x01, 0x02]); // CBOR array(2)[1,2]
      correlations.set(corr, payload);
      const expiresAt = new Date(Date.now() + 60 * 1000).toISOString();
      return {
        correlation_id: corr,
        payload_cbor_base64: Buffer.from(payload).toString("base64"),
        expires_at: expiresAt,
      };
    }
    case "mnemonic_recall": {
      const query = typeof args.query === "string" ? args.query : "";
      return {
        hits: [
          {
            attestation_id: "att-mock-1",
            content: `recalled for: ${query}`,
            similarity: 0.97,
            signed_at: new Date().toISOString(),
            tags: Array.isArray(args.tags) ? args.tags : [],
          },
        ],
        total: 1,
      };
    }
    case "mnemonic_verify": {
      const id =
        typeof args.attestation_id === "string" ? args.attestation_id : "";
      if (id.startsWith("tampered-")) {
        return {
          status: "tampered",
          signer: "MockSigner1",
          reason: "content_hash mismatch",
        };
      }
      if (id.startsWith("missing-")) {
        return { status: "not_found" };
      }
      return {
        status: "verified",
        signer: "MockSigner1",
        arweave_tx: "mock-arweave",
        solana_tx: "mock-solana",
      };
    }
    case "mnemonic_prove_identity":
      return {
        pubkey: "MockProver1",
        challenge: typeof args.challenge === "string" ? args.challenge : "",
        signature: "mock-signature",
      };
    default:
      return { error: `unknown tool: ${name}` };
  }
}
