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
 * Solana / `KeypairJson` 64-element JSON byte-array shape:
 * `[n0, n1, ..., n63]` where each `n` is a 1-3 digit decimal byte.
 *
 * T11 audit (A09): wasm-bindgen errors and stored-keypair payloads
 * (`/api/cli-bootstrap/redeem`) carry this shape. The hex regex above
 * doesn't catch it. We intentionally over-match (any `\d{1,3}`, not
 * strictly 0-255) — values >255 only appear in this shape if the array
 * is malformed, in which case redacting is still the safer outcome.
 */
const SOLANA_KEYPAIR_ARRAY_RE = /\[\s*(?:\d{1,3}\s*,\s*){63}\d{1,3}\s*\]/g;

/**
 * Replace JWT-shaped runs, 128-hex secret runs, and 64-element
 * Solana-keypair JSON arrays with redaction placeholders.
 *
 * Idempotent: running it twice on the same string produces the same output.
 * Safe on `undefined` / `null` — coerced to empty string.
 *
 * @param input - Arbitrary value; non-string inputs are coerced via
 *                `String(...)` before redaction. `null` / `undefined`
 *                map to the empty string.
 * @returns The input with all JWT-shaped substrings, 128-hex-char
 *          secret runs, and 64-element decimal-byte arrays replaced.
 */
export function redactJWT(input: unknown): string {
  if (input === undefined || input === null) return "";
  const s = typeof input === "string" ? input : String(input);
  return s
    .replace(JWT_RE, "[REDACTED-JWT]")
    .replace(HEX_SECRET_RE, "[REDACTED-SECRET]")
    .replace(SOLANA_KEYPAIR_ARRAY_RE, "[REDACTED-KEYPAIR]");
}

/**
 * Base class for every error thrown by the SDK. Consumers can
 * `instanceof MnemonicError` to catch any SDK-originated failure.
 *
 * @param message - Human-readable message; runs through {@link redactJWT}
 *                  before being stored on the `Error`.
 * @param cause   - Optional underlying cause (Error or any value). Not
 *                  redacted — developer-facing diagnostic chain only.
 */
export class MnemonicError extends Error {
  readonly cause?: unknown;
  constructor(message: string, cause?: unknown) {
    super(redactJWT(message));
    this.name = "MnemonicError";
    if (cause !== undefined) this.cause = cause;
  }
}

/**
 * Thrown for 401 / 403, missing or expired JWT, signer mismatch, OAuth
 * state / redirect_uri mismatch. Maps to CLI exit code `4`.
 *
 * @param message - Human-readable message (will be JWT-redacted).
 * @param cause   - Optional underlying cause.
 */
export class AuthError extends MnemonicError {
  constructor(message: string, cause?: unknown) {
    super(message, cause);
    this.name = "AuthError";
  }
}

/**
 * Thrown for 5xx responses, network failures, and malformed server
 * payloads. Maps to CLI exit code `2`.
 *
 * @param message - Human-readable message (will be JWT-redacted).
 * @param status  - Optional HTTP status code from the failing response.
 * @param cause   - Optional underlying cause.
 */
export class ServerError extends MnemonicError {
  readonly status?: number;
  constructor(message: string, status?: number, cause?: unknown) {
    super(message, cause);
    this.name = "ServerError";
    if (status !== undefined) this.status = status;
  }
}

/**
 * Thrown when a verification step fails — e.g. the COSE envelope's
 * content_hash disagrees with what the server stored, or
 * `verify(<id>)` returned `tampered`. Maps to CLI exit code `3`.
 *
 * @param message - Human-readable message (will be JWT-redacted).
 * @param cause   - Optional underlying cause.
 */
export class IntegrityError extends MnemonicError {
  constructor(message: string, cause?: unknown) {
    super(message, cause);
    this.name = "IntegrityError";
  }
}

/**
 * Thrown for caller-side bad input — empty content, malformed UUID,
 * missing keypair before `signMemory`. Maps to CLI exit code `1`.
 *
 * @param message - Human-readable message (will be JWT-redacted).
 * @param cause   - Optional underlying cause.
 */
export class UserError extends MnemonicError {
  constructor(message: string, cause?: unknown) {
    super(message, cause);
    this.name = "UserError";
  }
}

/**
 * Thrown by `Keypair.fromJSON` when given a stub-shaped identity.json
 * (pubkey + keychain_ref, no secret bytes). Signals the caller to
 * resolve the secret via an OS keychain instead of from disk.
 *
 * @param pubkey_base58 - The Ed25519 public key from the stub.
 * @param keychain_ref  - The OS keychain entry name to look up.
 */
export class IdentityRequiresKeystore extends Error {
  constructor(
    public readonly pubkey_base58: string,
    public readonly keychain_ref: string,
  ) {
    super(
      `identity.json is a stub referencing keychain entry '${keychain_ref}'; load via OS keychain`,
    );
    this.name = "IdentityRequiresKeystore";
  }
}
