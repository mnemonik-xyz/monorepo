// Google sign-in client. Drives RFC 7636 PKCE S256 against our server's
// `/oauth/google/start` via `chrome.identity.launchWebAuthFlow`, then
// redeems the returned auth code at `POST /oauth/token` for the
// server-issued JWT (Decision 5 of `work/chrome-extension/decisions.md`).
//
// Key invariants:
//
//  1. **No Google client_secret in the extension.** The redirect URI is
//     `chrome.identity.getRedirectURL('google')` and the server holds the
//     Google secret. The server validates Google's id_token + binds
//     `google_sub` to a server-minted JWT.
//
//  2. **State must round-trip exactly.** RFC 7636 §4.4 binds the PKCE
//     verifier to the state value; the callback's `state` MUST equal the
//     start request's `state` or we throw BEFORE the token-exchange POST.
//     Drives the `pkce_state_mismatch_rejects` TDD anchor.
//
//  3. **PKCE inputs match the project standard.** 32 random bytes →
//     base64url verifier (RFC 7636 §4.1, ~43 chars); SHA-256(verifier) →
//     base64url challenge (§4.2). Mirrors `packages/sdk/src/oauth.ts`.
//
// `signInWithGoogle` is the single public entry point. The popup awaits
// it; on success it persists a `Session` via `session.ts` and asks
// `lookup.ts` whether this Google account already has a linked pubkey.

import {
  AuthError,
  type GoogleProfile,
  type GoogleSignInResult,
} from "./types.js";

/** Public client id label sent on `/oauth/google/start`. The server
 *  treats this as informational; the redirect_uri is the real binding. */
export const EXTENSION_CLIENT_ID = "mnemonic-extension";

/** Default MCP server origin. Tests inject their own via `serverBaseUrl`. */
export const DEFAULT_SERVER_ORIGIN = "https://mc.mnemonik.xyz";

/** PKCE method — S256 only (server enforces, RFC 7636 §4.2). */
const PKCE_METHOD = "S256";

/** Path identifier passed to `chrome.identity.getRedirectURL`. The
 *  resulting URL is `https://<extension-id>.chromiumapp.org/google`. */
const REDIRECT_PATH = "google";

/**
 * Test-only seam: override the `chrome.identity` surface. Production code
 * leaves the field undefined and the runtime resolves to the real
 * `chrome.identity.*` APIs at call time. Centralising the indirection
 * here avoids monkey-patching the global `chrome` object in tests.
 */
export interface ChromeIdentityShim {
  getRedirectURL: (path: string) => string;
  /** Resolves to the callback URL on success, `undefined` on
   *  user-cancellation, OR rejects with an Error on
   *  `chrome.runtime.lastError`. The default implementation maps the
   *  callback-style API into this Promise shape. */
  launchWebAuthFlow: (details: {
    url: string;
    interactive: boolean;
  }) => Promise<string | undefined>;
}

export interface SignInWithGoogleInput {
  /** Origin of the MCP server, e.g. `https://mc.mnemonik.xyz`. No
   *  trailing slash required — the URL constructor handles both. */
  serverBaseUrl?: string;
  /** Test-only: override `chrome.identity.*`. Production callers leave
   *  this unset and the runtime resolves to the real APIs. */
  identity?: ChromeIdentityShim;
  /** Test-only: override `fetch`. Production callers leave this unset
   *  and the runtime uses the global `fetch`. */
  fetchImpl?: typeof fetch;
}

/**
 * Drive the full Google sign-in flow.
 *
 * Throws an {@link AuthError} on every failure path. The popup
 * branches on `error.message` to distinguish user-cancellation
 * ("Google sign-in was cancelled by the user") from real failures.
 */
export async function signInWithGoogle(
  input: SignInWithGoogleInput = {},
): Promise<GoogleSignInResult> {
  const serverBaseUrl = (input.serverBaseUrl ?? DEFAULT_SERVER_ORIGIN).replace(
    /\/+$/u,
    "",
  );
  const identity = input.identity ?? defaultIdentityShim();
  const fetchImpl = input.fetchImpl ?? globalThis.fetch.bind(globalThis);

  const verifier = generateCodeVerifier();
  const challenge = await pkceChallenge(verifier);
  const state = randomState();
  const redirectUri = identity.getRedirectURL(REDIRECT_PATH);
  if (typeof redirectUri !== "string" || redirectUri.length === 0) {
    throw new AuthError("chrome.identity.getRedirectURL returned empty");
  }

  const startUrl = buildAuthorizeUrl({
    serverBaseUrl,
    challenge,
    state,
    redirectUri,
  });

  let responseUrl: string | undefined;
  try {
    responseUrl = await identity.launchWebAuthFlow({
      url: startUrl,
      interactive: true,
    });
  } catch (cause) {
    if (cause instanceof AuthError) throw cause;
    throw new AuthError("launchWebAuthFlow failed", { cause });
  }
  if (responseUrl === undefined || responseUrl === "") {
    // chrome.identity surfaces user-cancellation by leaving responseUrl
    // undefined. Mirror that to a distinct error message so the popup
    // can render a silent "Sign-in cancelled" toast.
    throw new AuthError("Google sign-in was cancelled by the user");
  }

  // Parse the chromiumapp.org callback. `chrome.identity` URL-encodes
  // every query param, so `URL.searchParams` is the right reader.
  let callback: URL;
  try {
    callback = new URL(responseUrl);
  } catch (cause) {
    throw new AuthError("oauth callback URL is not a valid URL", { cause });
  }
  const errorParam = callback.searchParams.get("error");
  if (errorParam) {
    throw new AuthError(`oauth: server returned error=${errorParam}`);
  }
  const code = callback.searchParams.get("code");
  const returnedState = callback.searchParams.get("state");
  if (!code) {
    throw new AuthError("oauth: callback missing code parameter");
  }
  // RFC 7636 §4.4 + RFC 6749 §10.12: validate `state` BEFORE any token
  // exchange. A mismatch indicates a CSRF or a swapped callback and the
  // flow MUST abort silently. Constant-time comparison so a timing oracle
  // cannot distinguish prefix-matching candidate states.
  if (!returnedState || !constantTimeEquals(state, returnedState)) {
    throw new AuthError("oauth: state mismatch");
  }

  const token = await exchangeCodeForToken({
    fetchImpl,
    serverBaseUrl,
    code,
    verifier,
    redirectUri,
  });

  // Parse the id_token payload (NOT verifying signature — the server
  // already did that against Google's JWKS). The `id_token` is optional
  // in the server's response shape (`GoogleTokenResponse`); when missing
  // we fall back to whatever the access-token JWT carries.
  const profile = extractProfile(token.idToken, token.accessToken);
  if (!profile.sub) {
    throw new AuthError("oauth: response missing google_sub claim");
  }

  return {
    jwt: token.accessToken,
    googleSub: profile.sub,
    profile,
  };
}

// ── PKCE primitives (exported for direct unit testing) ─────────────────────

/** RFC 7636 §4.1: 32 random bytes, base64url-encoded (43 chars, no pad). */
export function generateCodeVerifier(): string {
  const buf = new Uint8Array(32);
  crypto.getRandomValues(buf);
  return base64UrlEncode(buf);
}

/** RFC 7636 §4.2: `base64url(SHA-256(verifier))`. */
export async function pkceChallenge(verifier: string): Promise<string> {
  const bytes = new TextEncoder().encode(verifier);
  // `crypto.subtle.digest` needs a `BufferSource` whose backing store is
  // an `ArrayBuffer`. Slicing produces a fresh, plain ArrayBuffer that
  // satisfies the strictest TS overload (matches the SDK pattern in
  // `packages/sdk/src/oauth.ts`).
  const ab = bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
  const digest = await crypto.subtle.digest("SHA-256", ab);
  return base64UrlEncode(new Uint8Array(digest));
}

/** 32-byte CSRF token, base64url-encoded. Decision 5 mandates ≥32 bytes. */
export function randomState(): string {
  const buf = new Uint8Array(32);
  crypto.getRandomValues(buf);
  return base64UrlEncode(buf);
}

/** RFC 4648 §5 base64url, no padding. */
function base64UrlEncode(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i] ?? 0);
  }
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/**
 * Constant-time string equality. Used for the state comparison so a
 * timing oracle cannot distinguish prefix-matching candidate states.
 * Different lengths return false without leaking which prefix matched.
 */
export function constantTimeEquals(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

// ── URL builders ────────────────────────────────────────────────────────────

export interface BuildAuthorizeUrlInput {
  serverBaseUrl: string;
  challenge: string;
  state: string;
  redirectUri: string;
}

/**
 * Build the `<serverBaseUrl>/oauth/google/start?...` URL handed to
 * `chrome.identity.launchWebAuthFlow`. Exported for direct testing of
 * the redirect-URI invariant (TDD anchor
 * `redirect_uri_uses_chrome_identity_helper`).
 */
export function buildAuthorizeUrl(input: BuildAuthorizeUrlInput): string {
  const u = new URL("/oauth/google/start", input.serverBaseUrl);
  u.searchParams.set("client_id", EXTENSION_CLIENT_ID);
  u.searchParams.set("redirect_uri", input.redirectUri);
  u.searchParams.set("code_challenge", input.challenge);
  u.searchParams.set("code_challenge_method", PKCE_METHOD);
  u.searchParams.set("state", input.state);
  return u.toString();
}

// ── chrome.identity default shim ────────────────────────────────────────────

/**
 * Build the production `chrome.identity` shim. Adapts the callback-style
 * `launchWebAuthFlow` API into the Promise shape the rest of this module
 * consumes. Surfaces `chrome.runtime.lastError` as a rejected promise so
 * the caller can branch on the AuthError message.
 *
 * Lookups go through `globalThis.chrome` (not the bare `chrome` global)
 * so that hosts without the `chrome.*` namespace (e.g. the vitest node
 * runner without our injected stub) fail loudly with a clear message.
 */
function defaultIdentityShim(): ChromeIdentityShim {
  const c = (globalThis as { chrome?: typeof chrome }).chrome;
  if (!c?.identity) {
    throw new AuthError("chrome.identity is unavailable in this context");
  }
  const id = c.identity;
  return {
    getRedirectURL: (path) => id.getRedirectURL(path),
    launchWebAuthFlow: (details) =>
      new Promise<string | undefined>((resolve, reject) => {
        try {
          id.launchWebAuthFlow(details, (responseUrl?: string) => {
            const lastError = c.runtime?.lastError;
            if (lastError) {
              reject(
                new AuthError(
                  `launchWebAuthFlow error: ${lastError.message ?? "unknown"}`,
                ),
              );
              return;
            }
            resolve(responseUrl);
          });
        } catch (cause) {
          reject(
            new AuthError(
              "chrome.identity.launchWebAuthFlow threw synchronously",
              { cause },
            ),
          );
        }
      }),
  };
}

// ── Token exchange ──────────────────────────────────────────────────────────

interface ExchangeInput {
  fetchImpl: typeof fetch;
  serverBaseUrl: string;
  code: string;
  verifier: string;
  redirectUri: string;
}

interface ExchangeOutput {
  accessToken: string;
  idToken: string | null;
  expiresAt: string;
}

async function exchangeCodeForToken(
  input: ExchangeInput,
): Promise<ExchangeOutput> {
  const tokenUrl = new URL("/oauth/token", input.serverBaseUrl).toString();
  let response: Response;
  try {
    response = await input.fetchImpl(tokenUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        grant_type: "authorization_code",
        code: input.code,
        code_verifier: input.verifier,
        redirect_uri: input.redirectUri,
        client_id: EXTENSION_CLIENT_ID,
      }),
    });
  } catch (cause) {
    throw new AuthError("oauth: token endpoint unreachable", { cause });
  }
  if (!response.ok) {
    throw new AuthError(
      `oauth: token endpoint returned ${String(response.status)}`,
    );
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    throw new AuthError("oauth: token endpoint returned non-JSON body", {
      cause,
    });
  }
  if (!body || typeof body !== "object") {
    throw new AuthError("oauth: token endpoint returned malformed body");
  }
  const obj = body as Record<string, unknown>;
  const accessToken =
    typeof obj.access_token === "string" ? obj.access_token : undefined;
  if (!accessToken) {
    throw new AuthError("oauth: token response missing access_token");
  }
  const idToken = typeof obj.id_token === "string" ? obj.id_token : null;
  const expiresIn =
    typeof obj.expires_in === "number" && Number.isFinite(obj.expires_in)
      ? obj.expires_in
      : 3600;
  const expiresAt = new Date(Date.now() + expiresIn * 1000).toISOString();
  return { accessToken, idToken, expiresAt };
}

// ── Profile extraction ──────────────────────────────────────────────────────

/**
 * Pull display fields out of the id_token payload (preferred) or fall
 * back to the access-token payload's `google_sub` claim.
 *
 * NEITHER token is signature-verified here — the server already validated
 * the id_token against Google's JWKS, and the access_token is HS256-signed
 * by us so any tampering would surface at the next API call.
 *
 * Exported for direct testing.
 */
export function extractProfile(
  idToken: string | null,
  accessToken: string,
): GoogleProfile {
  if (idToken) {
    const claims = parseJwtPayload(idToken);
    if (claims) {
      const sub = typeof claims.sub === "string" ? claims.sub : "";
      const email = typeof claims.email === "string" ? claims.email : "";
      const emailVerified = claims.email_verified === true;
      const name = typeof claims.name === "string" ? claims.name : "";
      const picture = typeof claims.picture === "string" ? claims.picture : "";
      if (sub) {
        return {
          sub,
          email,
          email_verified: emailVerified,
          name,
          picture,
        };
      }
    }
  }
  // Fallback: server JWT carries `google_sub`. No display fields here.
  const claims = parseJwtPayload(accessToken);
  const fallbackSub =
    typeof claims?.google_sub === "string" ? claims.google_sub : "";
  return {
    sub: fallbackSub,
    email: "",
    email_verified: false,
    name: "",
    picture: "",
  };
}

/**
 * Decode the second segment of a JWT to a JSON object. Returns `null`
 * for any malformed input — callers fall back to the access-token path.
 * No signature verification (see `extractProfile` for rationale).
 *
 * Exported for direct testing.
 */
export function parseJwtPayload(jwt: string): Record<string, unknown> | null {
  const parts = jwt.split(".");
  if (parts.length !== 3) return null;
  const seg = parts[1];
  if (!seg) return null;
  try {
    const padded =
      seg.replace(/-/g, "+").replace(/_/g, "/") +
      "===".slice((seg.length + 3) % 4);
    const bin = atob(padded);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const text = new TextDecoder("utf-8").decode(bytes);
    const parsed = JSON.parse(text);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }
    return parsed as Record<string, unknown>;
  } catch {
    return null;
  }
}
