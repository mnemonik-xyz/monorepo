// @mnemonik-xyz/sdk — OAuth 2.1 + PKCE primitives.
//
// Pure ESM, Web APIs only. No platform-builtin module imports.
//
// This module owns the SDK side of the OAuth 2.1 + PKCE handshake. It does
// NOT spin up an HTTP server — that's the CLI's responsibility (Task 5),
// or a Chrome extension's (`chrome.identity.launchWebAuthFlow`), or any
// other host that wants to drive the redirect.
//
// Decision 5 (RFC 7636 §4.4 + RFC 8252 §7): the PKCE state value is bound
// to the verifier AND the redirect_uri at authorize-time. At code-exchange
// time we validate that both match the originally-issued tuple before
// issuing the HTTP request. A mismatch terminates the flow.

import { AuthError } from "./errors.js";

/** Module-level pending-auth-session store, keyed by `sessionId` (UUIDv4). */
export interface PendingAuthSession {
  verifier: string;
  state: string;
  redirectUri: string;
  sessionId: string;
  /** Wall-clock millisecond timestamp at which this session was inserted.
   *  Used to enforce `PENDING_SESSION_TTL_MS`. */
  createdAt: number;
}

/**
 * Maximum age of a pending auth session, in milliseconds. Sessions older
 * than this are dropped on access (and on insert-time pruning). Set to
 * 10 minutes — long enough for a slow human to drive a browser through
 * the IdP, short enough that abandoned flows don't accumulate.
 *
 * The exact value is not mandated by tech-spec; chosen to mirror the
 * typical server-side PKCE `code` TTL (RFC 7636 §6.1 implementation note).
 */
export const PENDING_SESSION_TTL_MS = 10 * 60 * 1000;

/**
 * Maximum number of concurrent pending auth sessions. When the cap is hit,
 * a new insert evicts the oldest entry (FIFO). Chosen as 100 — far above
 * any realistic concurrent-flow count, far below the threshold at which
 * unbounded growth becomes a memory-pressure problem in long-lived hosts
 * (Chrome extensions, agent frameworks).
 */
export const PENDING_SESSION_MAX = 100;

/**
 * In-memory map of pending auth sessions, keyed by `sessionId` (UUIDv4).
 *
 * IMPORTANT: callers should prefer the internal `setSession` / `getSession`
 * / `deleteSession` helpers (used by `buildAuthorizeUrl` and
 * `exchangeCodeForToken` already), which enforce the TTL + size cap.
 * Direct mutation of this Map bypasses those guards and is reserved for
 * tests.
 *
 * Caller manages the OAuth lifecycle by passing `sessionId` back to
 * {@link exchangeCodeForToken}.
 */
export const pendingAuthSessions = new Map<string, PendingAuthSession>();

/** Drop entries older than {@link PENDING_SESSION_TTL_MS}. O(n). */
function pruneExpiredSessions(now: number): void {
  for (const [id, entry] of pendingAuthSessions) {
    if (now - entry.createdAt >= PENDING_SESSION_TTL_MS) {
      pendingAuthSessions.delete(id);
    }
  }
}

/**
 * Insert a session, enforcing TTL pruning + FIFO eviction at the size
 * cap. Map iteration order is insertion order (ECMA-262 §24.1.3.6) so
 * the first key returned by `keys()` is the oldest entry.
 *
 * @param id    - Session identifier (UUIDv4).
 * @param value - The session payload to store.
 * @returns void.
 */
export function setSession(id: string, value: PendingAuthSession): void {
  const now = Date.now();
  pruneExpiredSessions(now);
  while (pendingAuthSessions.size >= PENDING_SESSION_MAX) {
    const oldest = pendingAuthSessions.keys().next();
    if (oldest.done) break;
    pendingAuthSessions.delete(oldest.value);
  }
  pendingAuthSessions.set(id, value);
}

/**
 * Look up a session, enforcing TTL on read.
 *
 * @param id - Session identifier (UUIDv4).
 * @returns The stored `PendingAuthSession`, or `undefined` if the session
 *          is missing OR expired (in which case it is also evicted).
 */
export function getSession(id: string): PendingAuthSession | undefined {
  const entry = pendingAuthSessions.get(id);
  if (!entry) return undefined;
  if (Date.now() - entry.createdAt >= PENDING_SESSION_TTL_MS) {
    pendingAuthSessions.delete(id);
    return undefined;
  }
  return entry;
}

/**
 * Remove a session by id.
 *
 * @param id - Session identifier (UUIDv4).
 * @returns `true` if a session was removed, `false` otherwise.
 */
export function deleteSession(id: string): boolean {
  return pendingAuthSessions.delete(id);
}

// ── helpers ─────────────────────────────────────────────────────────────────

/** RFC 4648 §5 base64url (no padding). */
function bytesToBase64Url(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i]!);
  }
  const b64 = btoa(bin);
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** UTF-8 encode via the global Web `TextEncoder`. */
function utf8(s: string): Uint8Array {
  return new TextEncoder().encode(s);
}

/** Cryptographically random N-byte buffer via Web Crypto. */
function randomBytes(n: number): Uint8Array {
  const buf = new Uint8Array(n);
  crypto.getRandomValues(buf);
  return buf;
}

/** RFC 4122 v4 UUID built from `crypto.getRandomValues` (Web Crypto only). */
function uuidv4(): string {
  // Prefer the platform implementation when available (Node ≥19, Bun, Deno,
  // browsers); fall back to a hand-rolled v4 for older runtimes.
  const c = crypto as Crypto & { randomUUID?: () => string };
  if (typeof c.randomUUID === "function") {
    return c.randomUUID();
  }
  const b = randomBytes(16);
  b[6] = (b[6]! & 0x0f) | 0x40; // version 4
  b[8] = (b[8]! & 0x3f) | 0x80; // RFC 4122 variant
  const hex = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(
    12,
    16
  )}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

// ── PKCE primitives ─────────────────────────────────────────────────────────

/**
 * Generate a PKCE `code_verifier`: 32 cryptographically random bytes,
 * base64url-encoded (43 chars, no padding). Per RFC 7636 §4.1 the
 * verifier MUST be 43–128 characters from the unreserved character set.
 *
 * @returns A fresh base64url-encoded verifier string.
 */
export function generatePkceVerifier(): string {
  return bytesToBase64Url(randomBytes(32));
}

/**
 * Compute the PKCE S256 `code_challenge` from a verifier:
 * `base64url(SHA-256(utf8(verifier)))`. RFC 7636 §4.2.
 *
 * @param verifier - A code_verifier produced by
 *                   {@link generatePkceVerifier}.
 * @returns The base64url-encoded S256 challenge.
 */
export async function pkceChallenge(verifier: string): Promise<string> {
  // `crypto.subtle.digest` is typed as accepting a `BufferSource` whose
  // backing store is `ArrayBuffer`; the `TextEncoder().encode(...)` return
  // type narrowed to `Uint8Array<ArrayBufferLike>` in TS 5.7+, which is not
  // strictly assignable. Pass `.buffer` to satisfy the strictest overload.
  const bytes = utf8(verifier);
  const digest = await crypto.subtle.digest(
    "SHA-256",
    bytes.buffer.slice(
      bytes.byteOffset,
      bytes.byteOffset + bytes.byteLength
    ) as ArrayBuffer
  );
  return bytesToBase64Url(new Uint8Array(digest));
}

/**
 * Generate a 32-byte random `state` value, base64url-encoded.
 *
 * @returns A fresh base64url-encoded state string suitable for use as
 *          the OAuth `state` parameter.
 */
export function randomState(): string {
  return bytesToBase64Url(randomBytes(32));
}

// ── public surface ─────────────────────────────────────────────────────────

export interface BuildAuthorizeUrlInput {
  baseUrl: string;
  clientId: string;
  redirectUri: string;
  scope?: string;
}

export interface BuildAuthorizeUrlResult {
  url: string;
  state: string;
  verifier: string;
  sessionId: string;
}

/**
 * Build an OAuth 2.1 authorize URL (PKCE S256). Stores the
 * `{verifier, state, redirectUri, sessionId}` tuple in
 * {@link pendingAuthSessions} so that {@link exchangeCodeForToken} can
 * validate the callback.
 *
 * `scope` defaults to `"mcp"`.
 *
 * @param input - `{baseUrl, clientId, redirectUri, scope?}` — `baseUrl`
 *                must be the origin of the OAuth server (path is appended).
 * @returns `{url, state, verifier, sessionId}`. The caller drives the
 *          browser to `url` and later passes `sessionId` (and the
 *          callback's `state` + `code`) into
 *          {@link exchangeCodeForToken}.
 */
export async function buildAuthorizeUrl(
  input: BuildAuthorizeUrlInput
): Promise<BuildAuthorizeUrlResult> {
  const { baseUrl, clientId, redirectUri } = input;
  const scope = input.scope ?? "mcp";

  const verifier = generatePkceVerifier();
  const challenge = await pkceChallenge(verifier);
  const state = randomState();
  const sessionId = uuidv4();

  const url = new URL("/oauth/authorize", baseUrl);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", clientId);
  url.searchParams.set("redirect_uri", redirectUri);
  url.searchParams.set("code_challenge", challenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("state", state);
  url.searchParams.set("scope", scope);

  setSession(sessionId, {
    verifier,
    state,
    redirectUri,
    sessionId,
    createdAt: Date.now(),
  });

  return {
    url: url.toString(),
    state,
    verifier,
    sessionId,
  };
}

export interface ExchangeCodeForTokenInput {
  baseUrl: string;
  code: string;
  state: string;
  redirectUri: string;
  sessionId: string;
}

export interface ExchangeCodeForTokenResult {
  jwt: string;
  expiresAt: string;
}

/**
 * Exchange an authorization `code` for a JWT at `POST /oauth/token`.
 *
 * Validates that `state` and `redirectUri` match the stored session
 * before issuing any HTTP request (Decision 5 — RFC 7636 §4.4 /
 * RFC 8252 §7). The session entry is removed on success OR on
 * validation failure (one-shot semantics).
 *
 * @param input - `{baseUrl, code, state, redirectUri, sessionId}` from the
 *                callback URL plus the `sessionId` returned by
 *                {@link buildAuthorizeUrl}.
 * @returns `{jwt, expiresAt}`. `expiresAt` falls back to "now + 1h" if
 *          the server omits the field.
 * @throws `AuthError` for session-not-found, state mismatch, redirect_uri
 *         mismatch, network failure, non-2xx token response, or malformed
 *         JSON / missing `jwt` claim.
 */
export async function exchangeCodeForToken(
  input: ExchangeCodeForTokenInput
): Promise<ExchangeCodeForTokenResult> {
  const { baseUrl, code, state, redirectUri, sessionId } = input;

  const session = getSession(sessionId);
  if (!session) {
    throw new AuthError("oauth: session not found or already consumed");
  }

  // Validate state + redirectUri match the originally-issued tuple BEFORE
  // any HTTP call — a mismatch is fatal and consumes the session.
  if (session.state !== state) {
    deleteSession(sessionId);
    throw new AuthError("oauth: state mismatch");
  }
  if (session.redirectUri !== redirectUri) {
    deleteSession(sessionId);
    throw new AuthError("oauth: redirect_uri mismatch");
  }

  const tokenUrl = new URL("/oauth/token", baseUrl).toString();
  let response: Response;
  try {
    response = await fetch(tokenUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        grant_type: "authorization_code",
        code,
        code_verifier: session.verifier,
        redirect_uri: session.redirectUri,
        client_id: "mnemonic-cli",
      }),
    });
  } catch (cause) {
    // Network / fetch-level failure — keep the session so the caller can
    // retry. Surface as AuthError with a redacted message.
    throw new AuthError("oauth: token endpoint unreachable", cause);
  }

  if (!response.ok) {
    deleteSession(sessionId);
    throw new AuthError(`oauth: token endpoint returned ${response.status}`);
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    deleteSession(sessionId);
    throw new AuthError("oauth: token endpoint returned non-JSON body", cause);
  }

  // Server-spec contract: { jwt, expires_at } or { access_token, expires_in }.
  // Match the existing mcp/src/oauth.rs response shape (jwt + expires_at).
  if (
    !body ||
    typeof body !== "object" ||
    typeof (body as { jwt?: unknown }).jwt !== "string"
  ) {
    deleteSession(sessionId);
    throw new AuthError("oauth: token endpoint returned malformed body");
  }

  const jwt = (body as { jwt: string }).jwt;
  const expiresAtRaw = (body as { expires_at?: unknown }).expires_at;
  const expiresAt =
    typeof expiresAtRaw === "string"
      ? expiresAtRaw
      : new Date(Date.now() + 3600 * 1000).toISOString();

  deleteSession(sessionId);
  return { jwt, expiresAt };
}

// ── JWT payload parsing (no signature verification) ─────────────────────────

export interface JwtPayload {
  sub: string;
  exp: number;
  iat: number;
}

/**
 * Decode a base64url-encoded segment to UTF-8 string. Wraps the underlying
 * `atob` call so that an invalid base64url alphabet surfaces as a plain
 * `Error` rather than leaking a host-specific `DOMException` /
 * `InvalidCharacterError` to the caller.
 */
function decodeBase64UrlToString(seg: string): string {
  const padded =
    seg.replace(/-/g, "+").replace(/_/g, "/") +
    "===".slice((seg.length + 3) % 4);
  let bin: string;
  try {
    bin = atob(padded);
  } catch (cause) {
    // atob throws DOMException("InvalidCharacterError") on out-of-alphabet
    // input. Rethrow as a plain Error so the caller's catch can wrap it
    // into a redacted AuthError without exposing host-specific exception
    // types to consumers.
    throw new Error("base64url: invalid character in segment", { cause });
  }
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder("utf-8").decode(bytes);
}

/**
 * Acceptable JWT `alg` header values for {@link parseJwtPayload}. Per
 * Decision 6 the server signs tokens with HS256; defence-in-depth at the
 * SDK boundary rejects `none` and any non-whitelisted algorithm so that a
 * spoofed or downgraded token never reaches host code (Chrome ext, agent
 * framework, CLI) without explicit opt-in.
 */
const JWT_ALLOWED_ALGS = new Set(["HS256"]);

/**
 * Parse a JWT's payload without verifying its signature (server is the
 * authority on signature validity). Used by the CLI's `--token <jwt>`
 * headless flow to surface obvious problems (missing claims, expired)
 * before persisting the token to disk.
 *
 * The header is decoded too, and `alg` is asserted against the allowlist
 * (currently `HS256` only — Decision 6). Tokens signed with `alg=none`,
 * `alg=RS256`, or any other algorithm are rejected even though signature
 * verification itself is server-side.
 *
 * @param jwt - A three-segment JWT string.
 * @returns The decoded `{sub, exp, iat}` payload.
 * @throws `AuthError` for malformed tokens, disallowed `alg`, missing
 *         required claims, or `exp` strictly in the past.
 */
export function parseJwtPayload(jwt: string): JwtPayload {
  if (typeof jwt !== "string" || jwt.length === 0) {
    throw new AuthError("jwt: empty or non-string token");
  }
  const parts = jwt.split(".");
  if (parts.length !== 3) {
    throw new AuthError("jwt: malformed (expected 3 segments)");
  }

  // Header — assert alg whitelist before touching the payload.
  let header: unknown;
  try {
    header = JSON.parse(decodeBase64UrlToString(parts[0]!));
  } catch (cause) {
    throw new AuthError("jwt: header is not valid JSON or base64url", cause);
  }
  if (!header || typeof header !== "object" || Array.isArray(header)) {
    throw new AuthError("jwt: header is not an object");
  }
  const algRaw = (header as Record<string, unknown>).alg;
  if (typeof algRaw !== "string") {
    throw new AuthError("jwt: header missing `alg`");
  }
  if (!JWT_ALLOWED_ALGS.has(algRaw)) {
    throw new AuthError(`jwt: alg must be HS256, got: ${algRaw}`);
  }

  // Payload — extract sub/exp/iat and validate freshness.
  let json: unknown;
  try {
    json = JSON.parse(decodeBase64UrlToString(parts[1]!));
  } catch (cause) {
    throw new AuthError("jwt: payload is not valid JSON or base64url", cause);
  }
  if (!json || typeof json !== "object" || Array.isArray(json)) {
    throw new AuthError("jwt: payload is not an object");
  }
  const obj = json as Record<string, unknown>;
  if (typeof obj.sub !== "string" || obj.sub.length === 0) {
    throw new AuthError("jwt: missing or empty `sub` claim");
  }
  if (typeof obj.exp !== "number" || !Number.isFinite(obj.exp)) {
    throw new AuthError("jwt: missing or invalid `exp` claim");
  }
  if (typeof obj.iat !== "number" || !Number.isFinite(obj.iat)) {
    throw new AuthError("jwt: missing or invalid `iat` claim");
  }

  const nowSec = Math.floor(Date.now() / 1000);
  if (obj.exp <= nowSec) {
    throw new AuthError("jwt: token expired");
  }

  return { sub: obj.sub, exp: obj.exp, iat: obj.iat };
}
