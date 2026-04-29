// Typed error hierarchy for @mnemonik-xyz/sdk.
//
// Per Decision 10: every thrown error message is run through `redactJWT`
// so JWT-shaped substrings never leak into stderr / stdout / logs.
//
// Decision 10 specifies SDK exit-code mapping:
//   AuthError      → CLI exit 4
//   ServerError    → CLI exit 2
//   IntegrityError → CLI exit 3
//   UserError      → CLI exit 1
//
// CLI translates these classes to exit codes; the SDK only throws.

/**
 * JWT-shape regex. Matches the prefix `eyJ` followed by ≥20 base64url chars,
 * which captures every realistic three-segment JWT header. We do NOT try to
 * match all three segments (`.<payload>.<sig>`) because partial JWTs (e.g.
 * the header alone in an error context) still leak structure.
 *
 * Greedy by design: `+` lets the matcher consume the entire JWT-shaped run
 * even if the surrounding text continues with more chars. The `g` flag
 * replaces every occurrence in a single pass.
 */
const JWT_RE = /eyJ[A-Za-z0-9_-]{20,}/g;

/** Hex-encoded ed25519 secret (64 bytes = 128 hex chars). */
const HEX_SECRET_RE = /\b[0-9a-fA-F]{128}\b/g;

/**
 * Replace JWT-shaped runs and 128-hex secret runs with `[REDACTED-JWT]` /
 * `[REDACTED-SECRET]`.
 *
 * Idempotent: running it twice on the same string produces the same output.
 * Safe on `undefined` / `null` — coerced to empty string.
 */
export function redactJWT(input: unknown): string {
  if (input === undefined || input === null) return "";
  const s = typeof input === "string" ? input : String(input);
  return s
    .replace(JWT_RE, "[REDACTED-JWT]")
    .replace(HEX_SECRET_RE, "[REDACTED-SECRET]");
}

/** Base class. All SDK errors extend this so consumers can `instanceof`-check. */
export class MnemonicError extends Error {
  readonly cause?: unknown;
  constructor(message: string, cause?: unknown) {
    super(redactJWT(message));
    this.name = "MnemonicError";
    if (cause !== undefined) this.cause = cause;
  }
}

/** 401 / 403 from server, missing/expired JWT, signer mismatch, etc. */
export class AuthError extends MnemonicError {
  constructor(message: string, cause?: unknown) {
    super(message, cause);
    this.name = "AuthError";
  }
}

/** 5xx, network failure, malformed JSON, etc. */
export class ServerError extends MnemonicError {
  readonly status?: number;
  constructor(message: string, status?: number, cause?: unknown) {
    super(message, cause);
    this.name = "ServerError";
    if (status !== undefined) this.status = status;
  }
}

/** Verification mismatch: COSE bytes don't match the server's content_hash. */
export class IntegrityError extends MnemonicError {
  constructor(message: string, cause?: unknown) {
    super(message, cause);
    this.name = "IntegrityError";
  }
}

/** Caller-side bad input: empty content, malformed UUID, etc. */
export class UserError extends MnemonicError {
  constructor(message: string, cause?: unknown) {
    super(message, cause);
    this.name = "UserError";
  }
}
