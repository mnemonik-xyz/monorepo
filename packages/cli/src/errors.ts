// CLI typed-error hierarchy + secret redaction + top-level handler.
//
// Decision 10 — every CLI error message is run through `redactSecrets` before
// being printed to stderr/stdout/log so JWT-shaped substrings and 64-byte hex
// secrets never leak into terminal scrollback.
//
// Exit codes (Decision 10):
//   0  ok
//   1  user error (bad input, missing identity, malformed JSON)
//   2  server error (5xx, network failure, malformed response)
//   3  integrity error (verify mismatch, content_hash drift)
//   4  auth error (401/403, expired JWT, signer mismatch)

import {
  AuthError as SdkAuthError,
  IntegrityError as SdkIntegrityError,
  ServerError as SdkServerError,
  UserError as SdkUserError,
} from "@mnemonik-xyz/sdk";

/** JWT-shape regex — same heuristic as SDK's `redactJWT`. */
const JWT_RE = /eyJ[A-Za-z0-9_-]{20,}/g;
/** 64-byte ed25519 secret (128 hex chars). */
const HEX_SECRET_RE = /\b[0-9a-fA-F]{128}\b/g;

/**
 * Replace JWT-shaped runs and 128-hex secret runs with redaction markers.
 * Idempotent: running it twice on the same string is a no-op.
 */
export function redactSecrets(input: unknown): string {
  if (input === undefined || input === null) return "";
  const s = typeof input === "string" ? input : String(input);
  return s
    .replace(JWT_RE, "[REDACTED-JWT]")
    .replace(HEX_SECRET_RE, "[REDACTED-SECRET]");
}

/** Base class — every CLI typed error carries an exit code. */
export class CliError extends Error {
  readonly exitCode: number;
  readonly cause?: unknown;
  constructor(message: string, exitCode: number, cause?: unknown) {
    super(redactSecrets(message));
    this.name = "CliError";
    this.exitCode = exitCode;
    if (cause !== undefined) this.cause = cause;
  }
}

/** Caller-side bad input (missing identity, bad flag, refuse-to-overwrite). */
export class UserError extends CliError {
  constructor(message: string, cause?: unknown) {
    super(message, 1, cause);
    this.name = "UserError";
  }
}

/** 5xx, network failure, malformed JSON from a remote server. */
export class ServerError extends CliError {
  constructor(message: string, cause?: unknown) {
    super(message, 2, cause);
    this.name = "ServerError";
  }
}

/** Verify mismatch / content_hash drift / chain broken. */
export class IntegrityError extends CliError {
  constructor(message: string, cause?: unknown) {
    super(message, 3, cause);
    this.name = "IntegrityError";
  }
}

/** 401/403, expired JWT, OAuth state mismatch. */
export class AuthError extends CliError {
  constructor(message: string, cause?: unknown) {
    super(message, 4, cause);
    this.name = "AuthError";
  }
}

/**
 * Map an SDK-thrown error to its CLI counterpart. Preserves the original
 * message (already redacted by the SDK) and the cause chain. Used at every
 * `try/catch` site that wraps an SDK call.
 */
export function fromSdkError(e: unknown): CliError {
  if (e instanceof CliError) return e;
  if (e instanceof SdkAuthError) return new AuthError(e.message, e);
  if (e instanceof SdkIntegrityError) return new IntegrityError(e.message, e);
  if (e instanceof SdkServerError) return new ServerError(e.message, e);
  if (e instanceof SdkUserError) return new UserError(e.message, e);
  if (e instanceof Error) return new UserError(e.message, e);
  return new UserError(String(e));
}

/**
 * Top-level catch-all. Printed-output discipline:
 *   - typed error → its `.message` (already redacted) on stderr, exit
 *     with the carried code.
 *   - unknown error → "Internal error" + redacted stack on stderr, exit 1.
 *
 * Never throws. Always exits the process.
 */
export function handleError(err: unknown): never {
  if (err instanceof CliError) {
    process.stderr.write(`${err.name}: ${err.message}\n`);
    process.exit(err.exitCode);
  }
  // Unknown — surface a redacted stack so we don't leak secrets via thrown
  // strings the SDK didn't wrap.
  const stack =
    err instanceof Error
      ? err.stack ?? err.message
      : typeof err === "string"
      ? err
      : JSON.stringify(err);
  process.stderr.write(`Internal error: ${redactSecrets(stack)}\n`);
  process.exit(1);
}
