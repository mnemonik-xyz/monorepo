// Shared types for the extension-side Google sign-in (T16) and downstream
// restore / escrow flows (T17+). Kept in a dedicated file so the
// implementation modules stay small and tests can import the surface
// without dragging in `chrome.*` references.
//
// The wire shapes here track the server contracts in
// `mcp/src/oauth/google.rs` (T14) and `mcp/src/oauth/mod.rs::TokenResponse`.
// Anything cosmetic (UI copy, error categorisation) lives in popup glue,
// not here.

/**
 * Decoded payload of a Google `id_token`. We only ever look at the fields
 * the extension needs — the rest of the JWT is opaque to us. Signature
 * verification happens **server-side**; the extension never validates the
 * Google signature directly.
 *
 * `sub` is REQUIRED and load-bearing: it's the stable Google account id
 * used as the key for `/oauth/google/lookup` and `/oauth/google/link`.
 * Other fields fall back to empty strings when the claim is absent or
 * non-string so the popup can render gracefully (no avatar, only email,
 * etc.).
 */
export interface GoogleProfile {
  /** Google account id (`sub` claim). Stable per-user, opaque. */
  sub: string;
  /** Email address. Empty string when absent. */
  email: string;
  /** Whether Google has verified the email. `false` when claim missing. */
  email_verified: boolean;
  /** Full name. Empty string when absent. */
  name: string;
  /** Profile picture URL. Empty string when absent. */
  picture: string;
}

/**
 * Successful return shape of `signInWithGoogle`.
 *
 * The caller (popup onboarding) immediately passes `jwt` to
 * {@link import("./lookup.js").lookupExisting} to discover whether this
 * Google account is already bound to a pubkey, and uses `profile` to
 * render the welcome screen.
 */
export interface GoogleSignInResult {
  /** Server-issued Bearer JWT (HS256). Carries `sub = <pubkey or
   *  google:sub>` and `google_sub` claim per
   *  `mcp/src/oauth/mod.rs::issue_jwt_with_google_sub`. */
  jwt: string;
  /** Convenience alias for `profile.sub`. */
  googleSub: string;
  /** Decoded `id_token` profile fields (best-effort; never trusted for
   *  authentication — server already validated the id_token against
   *  Google's JWKS in T14). */
  profile: GoogleProfile;
  /** Wall-clock ms at which the access token expires. Derived from the
   *  server's `expires_in` (defaulting to 1h when absent). Callers
   *  populate `Session.jwtExpiresAt` from this so the source of truth
   *  for token lifetime is the server response, not a client-parsed
   *  JWT `exp` claim. */
  jwtExpiresAt: number;
}

/**
 * Persisted session blob — written to `chrome.storage.local` under the
 * key `session.v1`. The `v1` suffix is intentional: any forward-
 * incompatible change to this shape MUST land as `session.v2` so a
 * half-upgraded extension does not crash on read.
 *
 * `jwtExpiresAt` is wall-clock ms (= JWT `exp` claim * 1000). The session
 * getter compares against `Date.now()` and surfaces `null` for expired
 * tokens — refresh-token support is in the backlog (Decision 5
 * mentions Phase 2 refresh flow).
 */
export interface Session {
  jwt: string;
  googleSub: string;
  profile: GoogleProfile;
  /** Wall-clock ms at which the JWT expires (= `exp * 1000`). */
  jwtExpiresAt: number;
  /** Wall-clock ms at which the session was minted. */
  signedInAt: number;
}

/** `chrome.storage.local` key for the persisted session blob. */
export const SESSION_STORAGE_KEY = "session.v1";

/**
 * Result of `POST /oauth/google/lookup`. Mirrors the server's
 * `LookupResponse` (`mcp/src/oauth/google.rs`):
 *
 *  - `existingPubkey` is non-null when this Google account is already
 *    linked to an Ed25519 identity. The popup branches on this: null →
 *    "set recovery passphrase" onboarding; non-null + escrow → "welcome
 *    back" restore; non-null + no escrow → manual import edge case.
 *
 *  - `escrowPresent` reports whether `key_escrow_blobs` has a row for the
 *    Google `sub`. Distinguishes the "linked from another device but never
 *    enabled escrow" edge case from the happy "linked + escrow" path.
 *
 *  - `linkChallenge` is the server-issued possession-proof nonce returned
 *    ONLY when `existingPubkey` is null. T17 consumes it as the message
 *    body for `POST /oauth/google/link`. Kept on the typed result so T17's
 *    link flow doesn't need a second HTTP round-trip to fetch it.
 */
export interface LookupResult {
  existingPubkey: string | null;
  escrowPresent: boolean;
  linkChallenge: string | null;
}

/**
 * Branded auth error. Surfaces at the popup boundary as "Google sign-in
 * failed: <message>"; the underlying `cause` is preserved so a developer
 * (`chrome://extensions` → service-worker console) can chase the
 * original `chrome.runtime.lastError` or `fetch` rejection.
 *
 * The class is intentionally simple — full PII redaction lives at the
 * UI layer, not in the error itself. Callers branch on `error.message`
 * to distinguish user-cancellation ("Google sign-in was cancelled by
 * the user") from real failures.
 */
export class AuthError extends Error {
  public override readonly name = "AuthError";
  public constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
  }
}
