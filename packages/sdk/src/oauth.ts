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
}

/** In-memory map of pending auth sessions. Caller manages the lifecycle by
 *  passing `sessionId` back to {@link exchangeCodeForToken}. */
export const pendingAuthSessions = new Map<string, PendingAuthSession>();

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
 * base64url-encoded (43 chars, no padding). Per RFC 7636 §4.1 the verifier
 * MUST be 43–128 characters from the unreserved character set.
 */
export function generatePkceVerifier(): string {
  return bytesToBase64Url(randomBytes(32));
}

/**
 * Compute the PKCE S256 `code_challenge` from a verifier:
 * `base64url(SHA-256(utf8(verifier)))`. RFC 7636 §4.2.
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

/** Generate a 32-byte random `state` value, base64url-encoded. */
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
 * Build an OAuth 2.1 authorize URL (PKCE S256). Stores the `{verifier, state,
 * redirectUri, sessionId}` tuple in {@link pendingAuthSessions} so that
 * {@link exchangeCodeForToken} can validate the callback.
 *
 * `scope` defaults to `"mcp"`.
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

  pendingAuthSessions.set(sessionId, {
    verifier,
    state,
    redirectUri,
    sessionId,
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
 * Validates that `state` and `redirectUri` match the stored session before
 * issuing any HTTP request (Decision 5 — RFC 7636 §4.4 / RFC 8252 §7).
 * The session entry is removed on success OR on validation failure
 * (one-shot semantics). Throws {@link AuthError} for any validation or
 * server failure.
 */
export async function exchangeCodeForToken(
  input: ExchangeCodeForTokenInput
): Promise<ExchangeCodeForTokenResult> {
  const { baseUrl, code, state, redirectUri, sessionId } = input;

  const session = pendingAuthSessions.get(sessionId);
  if (!session) {
    throw new AuthError("oauth: session not found or already consumed");
  }

  // Validate state + redirectUri match the originally-issued tuple BEFORE
  // any HTTP call — a mismatch is fatal and consumes the session.
  if (session.state !== state) {
    pendingAuthSessions.delete(sessionId);
    throw new AuthError("oauth: state mismatch");
  }
  if (session.redirectUri !== redirectUri) {
    pendingAuthSessions.delete(sessionId);
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
    pendingAuthSessions.delete(sessionId);
    throw new AuthError(`oauth: token endpoint returned ${response.status}`);
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    pendingAuthSessions.delete(sessionId);
    throw new AuthError("oauth: token endpoint returned non-JSON body", cause);
  }

  // Server-spec contract: { jwt, expires_at } or { access_token, expires_in }.
  // Match the existing mcp/src/oauth.rs response shape (jwt + expires_at).
  if (
    !body ||
    typeof body !== "object" ||
    typeof (body as { jwt?: unknown }).jwt !== "string"
  ) {
    pendingAuthSessions.delete(sessionId);
    throw new AuthError("oauth: token endpoint returned malformed body");
  }

  const jwt = (body as { jwt: string }).jwt;
  const expiresAtRaw = (body as { expires_at?: unknown }).expires_at;
  const expiresAt =
    typeof expiresAtRaw === "string"
      ? expiresAtRaw
      : new Date(Date.now() + 3600 * 1000).toISOString();

  pendingAuthSessions.delete(sessionId);
  return { jwt, expiresAt };
}

// ── JWT payload parsing (no signature verification) ─────────────────────────

export interface JwtPayload {
  sub: string;
  exp: number;
  iat: number;
}

/** Decode a base64url-encoded segment to UTF-8 string. */
function decodeBase64UrlToString(seg: string): string {
  const padded =
    seg.replace(/-/g, "+").replace(/_/g, "/") +
    "===".slice((seg.length + 3) % 4);
  const bin = atob(padded);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder("utf-8").decode(bytes);
}

/**
 * Parse a JWT's payload without verifying its signature (server is the
 * authority on signature validity). Used by the CLI's `--token <jwt>`
 * headless flow to surface obvious problems (missing claims, expired)
 * before persisting the token to disk.
 *
 * Throws {@link AuthError} for malformed tokens, missing required claims,
 * or `exp` strictly in the past.
 */
export function parseJwtPayload(jwt: string): JwtPayload {
  if (typeof jwt !== "string" || jwt.length === 0) {
    throw new AuthError("jwt: empty or non-string token");
  }
  const parts = jwt.split(".");
  if (parts.length !== 3) {
    throw new AuthError("jwt: malformed (expected 3 segments)");
  }
  let json: unknown;
  try {
    json = JSON.parse(decodeBase64UrlToString(parts[1]!));
  } catch (cause) {
    throw new AuthError("jwt: payload is not valid JSON", cause);
  }
  if (!json || typeof json !== "object") {
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
