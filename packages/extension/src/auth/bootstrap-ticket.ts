// Extension-bootstrap ticket flow (T15 server, FW-S-01 fix).
//
// The MCP server issues two distinct JWTs from the same signing secret:
//
//   - `aud=mcp`        — minted by `POST /oauth/token` (T14 Google OAuth
//                        completion). Authenticates `/oauth/*` endpoints
//                        and the `POST /api/extension-bootstrap/issue`
//                        handshake itself.
//   - `aud=extension`  — minted by `GET /api/extension-bootstrap/redeem/
//                        :ticket`. Authenticates `/api/key-escrow*`
//                        endpoints (see `mcp/src/escrow.rs::
//                        extract_extension_claims`).
//
// This module performs the handshake. Given an `aud=mcp` JWT it (1) posts
// to `/api/extension-bootstrap/issue` to obtain a one-time UUID ticket,
// (2) gets `/api/extension-bootstrap/redeem/:ticket` (NO auth — the ticket
// IS the capability) to receive the fresh `aud=extension` JWT. The result
// is cached in `Session.extensionJwt`/`extensionJwtExpiresAt` so subsequent
// calls reuse the token until expiry.
//
// The bootstrap endpoints are documented in `mcp/src/escrow.rs:435-515`.
//
// WIRE FORMAT
//   POST /api/extension-bootstrap/issue
//     Authorization: Bearer <aud=mcp JWT>
//     → 200 { ticket_id: "<uuid>" }
//   GET  /api/extension-bootstrap/redeem/:ticket
//     (no auth)
//     → 200 { access_token: "<aud=extension JWT>", aud: "extension",
//             expires_in: <seconds> }
//
// Tickets have a 10-minute TTL server-side and are single-use; the
// redeem endpoint returns an identical 404 body for unknown / expired /
// already-consumed tickets so attackers probing the namespace cannot
// distinguish failure modes.

import { AuthError, type Session } from "./types.js";
import { DEFAULT_SERVER_ORIGIN } from "./google-oauth.js";
import { getSession, setSession } from "./session.js";

/** Safety margin for cached extension-JWT freshness. We treat a token as
 *  expired if it has less than 60 seconds remaining, so an in-flight
 *  keyEscrow call doesn't fail mid-request because of clock skew. */
const EXTENSION_JWT_SAFETY_MARGIN_MS = 60_000;

/** Default minted-JWT lifetime when the server response omits
 *  `expires_in` (defensive — production server always sets it). */
const DEFAULT_EXTENSION_JWT_LIFETIME_SECS = 3600;

export interface BootstrapTicketOptions {
  serverOrigin?: string;
  fetchImpl?: typeof fetch;
  now?: () => number;
}

interface RedeemedJwt {
  access_token: string;
  expires_in: number;
}

function bootstrapIssueUrl(opts: BootstrapTicketOptions | undefined): string {
  const origin = (opts?.serverOrigin ?? DEFAULT_SERVER_ORIGIN).replace(
    /\/+$/u,
    "",
  );
  return new URL("/api/extension-bootstrap/issue", origin).toString();
}

function bootstrapRedeemUrl(
  opts: BootstrapTicketOptions | undefined,
  ticket: string,
): string {
  const origin = (opts?.serverOrigin ?? DEFAULT_SERVER_ORIGIN).replace(
    /\/+$/u,
    "",
  );
  return new URL(
    `/api/extension-bootstrap/redeem/${encodeURIComponent(ticket)}`,
    origin,
  ).toString();
}

/**
 * `POST /api/extension-bootstrap/issue`. Requires the `aud=mcp` JWT as a
 * Bearer. Returns the one-time UUID ticket.
 */
export async function issueTicket(
  jwtMcp: string,
  opts: BootstrapTicketOptions = {},
): Promise<string> {
  if (typeof jwtMcp !== "string" || jwtMcp.length === 0) {
    throw new AuthError("issueTicket: jwt_mcp is required");
  }
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  let response: Response;
  try {
    response = await fetchImpl(bootstrapIssueUrl(opts), {
      method: "POST",
      headers: {
        authorization: `Bearer ${jwtMcp}`,
        "content-type": "application/json",
      },
      body: "{}",
    });
  } catch (cause) {
    throw new AuthError("issueTicket: server unreachable", { cause });
  }
  if (response.status === 401) {
    throw new AuthError("issueTicket: server rejected aud=mcp JWT (401)");
  }
  if (response.status === 429) {
    throw new AuthError("issueTicket: too many pending tickets (429)");
  }
  if (!response.ok) {
    throw new AuthError(
      `issueTicket: server returned ${String(response.status)}`,
    );
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    throw new AuthError("issueTicket: response is not valid JSON", { cause });
  }
  if (!body || typeof body !== "object") {
    throw new AuthError("issueTicket: response is not an object");
  }
  const ticket = (body as Record<string, unknown>).ticket_id;
  if (typeof ticket !== "string" || ticket.length === 0) {
    throw new AuthError("issueTicket: response.ticket_id missing");
  }
  return ticket;
}

/**
 * `GET /api/extension-bootstrap/redeem/:ticket`. NO auth — the ticket
 * is the capability. Returns the `aud=extension` JWT.
 */
export async function redeemTicket(
  ticket: string,
  opts: BootstrapTicketOptions = {},
): Promise<RedeemedJwt> {
  if (typeof ticket !== "string" || ticket.length === 0) {
    throw new AuthError("redeemTicket: ticket is required");
  }
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  let response: Response;
  try {
    response = await fetchImpl(bootstrapRedeemUrl(opts, ticket), {
      method: "GET",
    });
  } catch (cause) {
    throw new AuthError("redeemTicket: server unreachable", { cause });
  }
  if (response.status === 404) {
    throw new AuthError(
      "redeemTicket: ticket not found, already consumed, or expired",
    );
  }
  if (!response.ok) {
    throw new AuthError(
      `redeemTicket: server returned ${String(response.status)}`,
    );
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    throw new AuthError("redeemTicket: response is not valid JSON", { cause });
  }
  if (!body || typeof body !== "object") {
    throw new AuthError("redeemTicket: response is not an object");
  }
  const o = body as Record<string, unknown>;
  if (typeof o.access_token !== "string" || o.access_token.length === 0) {
    throw new AuthError("redeemTicket: response.access_token missing");
  }
  const exp =
    typeof o.expires_in === "number" && Number.isFinite(o.expires_in)
      ? Math.floor(o.expires_in)
      : DEFAULT_EXTENSION_JWT_LIFETIME_SECS;
  return { access_token: o.access_token, expires_in: exp };
}

/**
 * Return a fresh `aud=extension` JWT for the session. If one is cached
 * and not yet expired (with safety margin), return it directly.
 * Otherwise drive the issue→redeem dance, persist the result back onto
 * the session, and return the new JWT.
 *
 * Throws {@link AuthError} on any transport / server failure. The
 * caller propagates this to the user (the popup onboarding renders an
 * actionable toast).
 */
export async function getOrRefreshExtensionJwt(
  session: Session,
  opts: BootstrapTicketOptions = {},
): Promise<string> {
  const now = opts.now ?? Date.now;
  if (
    typeof session.extensionJwt === "string" &&
    session.extensionJwt.length > 0 &&
    typeof session.extensionJwtExpiresAt === "number" &&
    session.extensionJwtExpiresAt - EXTENSION_JWT_SAFETY_MARGIN_MS > now()
  ) {
    return session.extensionJwt;
  }
  const ticket = await issueTicket(session.jwt, opts);
  const redeemed = await redeemTicket(ticket, opts);
  const expiresAt = now() + redeemed.expires_in * 1000;
  const next: Session = {
    ...session,
    extensionJwt: redeemed.access_token,
    extensionJwtExpiresAt: expiresAt,
  };
  try {
    await setSession(next);
  } catch {
    // Persistence failure is non-fatal — the JWT is still returned so
    // the in-flight call proceeds; the next call will simply mint
    // another ticket. The user-visible behaviour is identical.
  }
  return redeemed.access_token;
}

/**
 * Convenience: load the persisted session and return its (refreshed)
 * extension JWT. Throws when no session is present — callers should
 * check `getSession()` first if they need a graceful no-session path.
 */
export async function getCurrentExtensionJwt(
  opts: BootstrapTicketOptions = {},
): Promise<string> {
  const session = await getSession();
  if (!session) {
    throw new AuthError(
      "getCurrentExtensionJwt: no Google session — sign in first",
    );
  }
  return getOrRefreshExtensionJwt(session, opts);
}
