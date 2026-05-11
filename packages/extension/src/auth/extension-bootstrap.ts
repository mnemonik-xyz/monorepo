// Extension-bootstrap client (T15 server-side counterpart). The server's
// `/api/key-escrow*` endpoints require a JWT minted with `aud=extension`;
// the popup's Google-sign-in flow only mints `aud=mcp`. This module
// runs the two-step handshake that exchanges an `aud=mcp` JWT for the
// short-lived `aud=extension` JWT every escrow / link / sign-callback
// call needs.
//
// Wire contract (mirrors `mcp/src/escrow.rs::extension_bootstrap_*`):
//
//   1. `POST /api/extension-bootstrap/issue`
//        headers: `Authorization: Bearer <aud=mcp JWT>`
//        response 200: `{ ticket_id: "<UUIDv4>" }`
//        response 429: `{ error: "too_many_pending_tickets" }`
//
//   2. `GET /api/extension-bootstrap/redeem/<ticket_id>`
//        headers: none (the UUID *is* the capability — single-use,
//        ~10-minute TTL)
//        response 200: `{ access_token: "<aud=extension JWT>",
//                         aud: "extension", expires_in: <secs> }`
//        response 404: ticket absent / expired / already consumed
//                      (deliberately ambiguous so probers cannot
//                      distinguish failure modes)
//
// Caching: the redeemed `aud=extension` JWT is cached in
// `chrome.storage.local` under {@link EXTENSION_SESSION_STORAGE_KEY} so a
// popup-close / SW-restart does not re-burn a ticket. {@link ensureExtensionJwt}
// returns the cached token if it has at least {@link CACHE_REFRESH_BUFFER_MS}
// of lifetime remaining; otherwise it re-handshakes. The cache is
// purely a latency / quota optimisation — the server already validates
// `exp` on every request.
//
// THREAT MODEL note: the `aud=extension` JWT is just as sensitive as
// the `aud=mcp` JWT (filesystem-compromise of the user's Chrome
// profile leaks both). We accept this per `auth/session.ts`'s threat
// model section; the per-audience separation is what the *server*
// uses to keep `aud=mcp` JWTs from being replayed against escrow
// endpoints, not what the client uses to harden against local
// attackers.

import { AuthError } from "./types.js";
import { DEFAULT_SERVER_ORIGIN } from "./google-oauth.js";

// ── Storage shim (mirrors auth/session.ts) ─────────────────────────────────

export interface StorageArea {
  get: (keys: string | string[]) => Promise<Record<string, unknown>>;
  set: (items: Record<string, unknown>) => Promise<void>;
  remove: (keys: string | string[]) => Promise<void>;
}

function defaultStorage(): StorageArea {
  const c = (globalThis as { chrome?: typeof chrome }).chrome;
  if (!c?.storage?.local) {
    throw new Error("chrome.storage.local is unavailable in this context");
  }
  return c.storage.local as unknown as StorageArea;
}

// ── Constants ──────────────────────────────────────────────────────────────

/** Storage key for the cached `aud=extension` JWT blob. Bumped if the
 *  shape ever changes (mirrors the `session.v1` convention). */
export const EXTENSION_SESSION_STORAGE_KEY = "extension_session.v1";

/** Refresh the cached JWT this many ms before its `expiresAt` so callers
 *  do not race a token expiry mid-request. 60s is large enough to absorb
 *  clock skew between the user's device and the server. */
const CACHE_REFRESH_BUFFER_MS = 60 * 1000;

// ── Types ──────────────────────────────────────────────────────────────────

/** Cached `aud=extension` JWT + its wall-clock expiry. Persisted under
 *  {@link EXTENSION_SESSION_STORAGE_KEY}. */
export interface ExtensionSession {
  /** `aud=extension` JWT. Hand directly to `Authorization: Bearer ...`. */
  jwt: string;
  /** Wall-clock ms at which the JWT expires (server's `expires_in` * 1000
   *  + `Date.now()` at receipt). */
  expiresAt: number;
}

export interface RedeemExtensionJwtOptions {
  /** Origin of the MCP server. Defaults to {@link DEFAULT_SERVER_ORIGIN}. */
  serverOrigin?: string;
  /** Test-only `fetch` override. Production uses the global. */
  fetchImpl?: typeof fetch;
}

export interface EnsureExtensionJwtOptions extends RedeemExtensionJwtOptions {
  /** Test-only storage override. Production uses `chrome.storage.local`. */
  storage?: StorageArea;
  /** Test-only clock override. */
  now?: () => number;
}

// ── Errors ─────────────────────────────────────────────────────────────────

/** Thrown when the server rate-limits the per-user pending-ticket cap. */
export class ExtensionBootstrapRateLimitError extends Error {
  public override readonly name = "ExtensionBootstrapRateLimitError";
  public constructor(
    message = "extension-bootstrap: too many pending tickets",
  ) {
    super(message);
  }
}

/** Thrown when the redeem path 404s (garbage / expired / already-consumed
 *  ticket — server returns identical bodies for all three). */
export class ExtensionBootstrapTicketNotFoundError extends Error {
  public override readonly name = "ExtensionBootstrapTicketNotFoundError";
  public constructor(message = "extension-bootstrap: ticket not found") {
    super(message);
  }
}

// ── HTTP helpers ───────────────────────────────────────────────────────────

function bootstrapBase(opts: RedeemExtensionJwtOptions | undefined): string {
  return (opts?.serverOrigin ?? DEFAULT_SERVER_ORIGIN).replace(/\/+$/u, "");
}

/**
 * Run the full two-step handshake and return the freshly-minted
 * `aud=extension` JWT. Does NOT touch storage — {@link ensureExtensionJwt}
 * wraps this with caching.
 *
 * Throws:
 *  - {@link ExtensionBootstrapRateLimitError} on 429 from `/issue`,
 *  - {@link ExtensionBootstrapTicketNotFoundError} on 404 from `/redeem`,
 *  - {@link AuthError} on transport / 401 / malformed-response / 5xx.
 */
export async function redeemExtensionJwt(
  mcpJwt: string,
  opts: RedeemExtensionJwtOptions = {},
): Promise<ExtensionSession> {
  if (typeof mcpJwt !== "string" || mcpJwt.length === 0) {
    throw new AuthError("redeemExtensionJwt: mcpJwt is required");
  }
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  const base = bootstrapBase(opts);

  // ── Step 1: POST /api/extension-bootstrap/issue ───────────────────────
  const issueUrl = new URL("/api/extension-bootstrap/issue", base).toString();
  let issueResp: Response;
  try {
    issueResp = await fetchImpl(issueUrl, {
      method: "POST",
      headers: {
        authorization: `Bearer ${mcpJwt}`,
        "content-type": "application/json",
      },
    });
  } catch (cause) {
    throw new AuthError("extension-bootstrap: server unreachable", { cause });
  }
  if (issueResp.status === 401) {
    throw new AuthError("extension-bootstrap: server rejected JWT (401)");
  }
  if (issueResp.status === 429) {
    throw new ExtensionBootstrapRateLimitError();
  }
  if (!issueResp.ok) {
    throw new AuthError(
      `extension-bootstrap: issue returned ${String(issueResp.status)}`,
    );
  }
  let issueBody: unknown;
  try {
    issueBody = await issueResp.json();
  } catch (cause) {
    throw new AuthError("extension-bootstrap: issue body not JSON", { cause });
  }
  if (!issueBody || typeof issueBody !== "object") {
    throw new AuthError("extension-bootstrap: issue body not an object");
  }
  const ticketId = (issueBody as Record<string, unknown>).ticket_id;
  if (typeof ticketId !== "string" || ticketId.length === 0) {
    throw new AuthError("extension-bootstrap: issue body missing ticket_id");
  }

  // ── Step 2: GET /api/extension-bootstrap/redeem/<ticket_id> ───────────
  // No Authorization header — the UUID is the capability.
  const redeemUrl = new URL(
    `/api/extension-bootstrap/redeem/${encodeURIComponent(ticketId)}`,
    base,
  ).toString();
  let redeemResp: Response;
  try {
    redeemResp = await fetchImpl(redeemUrl, { method: "GET" });
  } catch (cause) {
    throw new AuthError("extension-bootstrap: redeem unreachable", { cause });
  }
  if (redeemResp.status === 404) {
    throw new ExtensionBootstrapTicketNotFoundError();
  }
  if (!redeemResp.ok) {
    throw new AuthError(
      `extension-bootstrap: redeem returned ${String(redeemResp.status)}`,
    );
  }
  let redeemBody: unknown;
  try {
    redeemBody = await redeemResp.json();
  } catch (cause) {
    throw new AuthError("extension-bootstrap: redeem body not JSON", { cause });
  }
  if (!redeemBody || typeof redeemBody !== "object") {
    throw new AuthError("extension-bootstrap: redeem body not an object");
  }
  const o = redeemBody as Record<string, unknown>;
  const accessToken = o.access_token;
  const expiresIn = o.expires_in;
  if (typeof accessToken !== "string" || accessToken.length === 0) {
    throw new AuthError(
      "extension-bootstrap: redeem body missing access_token",
    );
  }
  const expiresInSecs =
    typeof expiresIn === "number" && Number.isFinite(expiresIn) && expiresIn > 0
      ? expiresIn
      : 3600;
  return {
    jwt: accessToken,
    expiresAt: Date.now() + expiresInSecs * 1000,
  };
}

// ── Cache helpers ──────────────────────────────────────────────────────────

function isPersistedExtensionSession(
  value: unknown,
): value is ExtensionSession {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  if (typeof v.jwt !== "string" || v.jwt.length === 0) return false;
  if (typeof v.expiresAt !== "number" || !Number.isFinite(v.expiresAt)) {
    return false;
  }
  return true;
}

/**
 * Read the cached `aud=extension` JWT, returning `null` when:
 *
 *   - no session has been written yet,
 *   - the stored blob fails shape validation,
 *   - the JWT will expire within {@link CACHE_REFRESH_BUFFER_MS}.
 *
 * Mirrors `auth/session.ts::getSession` — expired rows are NOT
 * auto-deleted; callers re-handshake and overwrite. (S7 mirror: we DO
 * fire-and-forget remove on the "no session" / "expired" branches so
 * the on-disk window stays in step with the policy window.)
 */
export async function getExtensionJwt(
  storage: StorageArea = defaultStorage(),
  now: () => number = Date.now,
): Promise<string | null> {
  let stored: Record<string, unknown>;
  try {
    stored = await storage.get(EXTENSION_SESSION_STORAGE_KEY);
  } catch {
    return null;
  }
  const raw = stored[EXTENSION_SESSION_STORAGE_KEY];
  if (!isPersistedExtensionSession(raw)) return null;
  if (raw.expiresAt - now() <= CACHE_REFRESH_BUFFER_MS) {
    // Best-effort cleanup of the stale row. Fire-and-forget — the next
    // {@link ensureExtensionJwt} call will overwrite anyway.
    void storage.remove(EXTENSION_SESSION_STORAGE_KEY).catch(() => {
      /* non-fatal */
    });
    return null;
  }
  return raw.jwt;
}

/**
 * Return a valid `aud=extension` JWT — from cache if fresh, otherwise
 * via a fresh {@link redeemExtensionJwt} handshake. Persists the new
 * blob to {@link EXTENSION_SESSION_STORAGE_KEY} on success so subsequent
 * calls hit the cache.
 *
 * Heavy callers (escrow / link / sign-callback) thread the `aud=mcp`
 * JWT in and get back the `aud=extension` JWT to use with their HTTP
 * call.
 */
export async function ensureExtensionJwt(
  mcpJwt: string,
  opts: EnsureExtensionJwtOptions = {},
): Promise<string> {
  if (typeof mcpJwt !== "string" || mcpJwt.length === 0) {
    throw new AuthError("ensureExtensionJwt: mcpJwt is required");
  }
  const storage = opts.storage ?? defaultStorage();
  const now = opts.now ?? Date.now;

  const cached = await getExtensionJwt(storage, now);
  if (cached !== null) return cached;

  // No fresh cache — do the handshake. The redeem options shape diverges
  // from the storage shape, so we narrow before forwarding.
  const redeemOpts: RedeemExtensionJwtOptions = {};
  if (opts.serverOrigin !== undefined)
    redeemOpts.serverOrigin = opts.serverOrigin;
  if (opts.fetchImpl !== undefined) redeemOpts.fetchImpl = opts.fetchImpl;
  const session = await redeemExtensionJwt(mcpJwt, redeemOpts);
  try {
    await storage.set({ [EXTENSION_SESSION_STORAGE_KEY]: session });
  } catch {
    // Persistence failures are non-fatal — the caller still gets the
    // freshly-minted JWT and the next call just re-handshakes.
  }
  return session.jwt;
}

/**
 * Clear the cached `aud=extension` JWT. Called on sign-out so the next
 * sign-in cannot reuse a stale token. Idempotent.
 */
export async function clearExtensionSession(
  storage: StorageArea = defaultStorage(),
): Promise<void> {
  try {
    await storage.remove(EXTENSION_SESSION_STORAGE_KEY);
  } catch {
    // Best-effort.
  }
}
