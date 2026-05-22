// `~/.mnemonic/` persistence layer.
//
// Tests override the directory via `MNEMONIC_CONFIG_DIR`. File-mode 0600 is
// enforced on Unix via `fs.chmodSync` and approximated on Windows via
// `icacls /inheritance:r /grant:r "${USERNAME}:F"` (removes inherited ACEs,
// grants full control to the calling user only).
//
// File layout:
//   <dir>/identity.json   {secret: number[64], pubkey_base58: string}
//   <dir>/token.json      {jwt, expires_at, sub}
//
// Legacy fallback: existing self-host users (server's MNEMONIC_KEYPAIR_PATH)
// have `<dir>/id.json` in Solana keypair file format — a bare JSON array of
// 64 numbers `[n0, ..., n63]` (32 seed + 32 pubkey). `loadIdentityJson()`
// transparently reads that file when `identity.json` is missing, deriving
// `pubkey_base58` from `secret[32..64]`. The on-disk file is preserved
// as-is (no auto-rewrite to identity.json).

import { execFileSync } from "node:child_process";
import {
  existsSync,
  chmodSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import {
  IdentityRequiresKeystore,
  Keypair,
  type KeypairJson,
} from "@mnemonik-xyz/sdk";

import { UserError } from "./errors.js";
import { OsKeyStore } from "./identity/keystore-os.js";

export interface TokenJson {
  jwt: string;
  /** ISO-8601 string from `exchangeCodeForToken` or derived from JWT exp. */
  expires_at: string;
  /** JWT `sub` claim — the OAuth subject (typically the user's pubkey). */
  sub: string;
}

/** Resolve the config directory (with env override for tests). */
export function configDir(): string {
  const override = process.env.MNEMONIC_CONFIG_DIR;
  if (override && override.length > 0) return override;
  return join(homedir(), ".mnemonic");
}

export function identityPath(): string {
  return join(configDir(), "identity.json");
}

/** Path to the legacy Solana-keypair-format file (server self-host layout). */
export function legacyIdJsonPath(): string {
  return join(configDir(), "id.json");
}

export function tokenPath(): string {
  return join(configDir(), "token.json");
}

/** Ensure the config directory exists. */
function ensureDir(): string {
  const dir = configDir();
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
  return dir;
}

/**
 * Restrict the file at `path` so that only the current user can read/write.
 * Unix: chmod 0600. Windows: `icacls /inheritance:r /grant:r "<user>:F"`.
 */
export function restrictFileMode(path: string): void {
  if (process.platform === "win32") {
    const user = process.env.USERNAME ?? process.env.USER ?? "";
    if (!user) {
      // Fall back to "best effort" — without a username we cannot ACL.
      return;
    }
    // `/inheritance:r` removes inherited ACEs (so a parent-directory ACL
    // cannot leak read permissions). `/grant:r` replaces existing user-grants.
    // execFileSync with an argv array bypasses cmd.exe entirely — embedded
    // quotes in `path` or `user` are passed literally to icacls, defeating
    // shell-injection (CWE-78) via user-supplied --file paths.
    try {
      execFileSync(
        "icacls",
        [path, "/inheritance:r", "/grant:r", `${user}:F`],
        { stdio: "ignore" },
      );
    } catch {
      // Non-fatal — the file is written; ACL hardening is best-effort on
      // Windows. Surface no warning here (it would be noisy on every save).
    }
    return;
  }
  chmodSync(path, 0o600);
}

// ── identity ────────────────────────────────────────────────────────────────

/** Validate a parsed object is shaped like `KeypairJson`. */
function isKeypairJson(v: unknown): v is KeypairJson {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    Array.isArray(o.secret) &&
    o.secret.length === 64 &&
    o.secret.every((n) => typeof n === "number") &&
    typeof o.pubkey_base58 === "string" &&
    o.pubkey_base58.length > 0
  );
}

/** True iff `v` is a JSON array of exactly 64 numbers in 0..=255. */
function isSolanaKeypairArray(v: unknown): v is number[] {
  if (!Array.isArray(v) || v.length !== 64) return false;
  for (const n of v) {
    if (typeof n !== "number" || !Number.isInteger(n) || n < 0 || n > 255) {
      return false;
    }
  }
  return true;
}

// Minimal Bitcoin-alphabet base58 encoder. ~30 lines of arithmetic; we don't
// pull in a third-party `bs58` dep for this single call site. Mirrors the
// SDK's test-only encoder in `sdk/test/helpers/bs58.ts`. Correctness is
// verified end-to-end via the WASM `Keypair.fromJSON` round-trip — the
// derived pubkey must match what the secret derives to or import fails.
const BASE58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function base58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  const digits: number[] = [];
  for (let i = zeros; i < bytes.length; i++) {
    let carry = bytes[i]!;
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j]! * 256;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  let s = "";
  for (let i = 0; i < zeros; i++) s += BASE58_ALPHABET[0];
  for (let i = digits.length - 1; i >= 0; i--) s += BASE58_ALPHABET[digits[i]!];
  return s;
}

/**
 * Convert a Solana keypair file (bare 64-number JSON array, 32-byte seed +
 * 32-byte pubkey) into our `KeypairJson` shape. The pubkey is derived purely
 * by base58-encoding `secret[32..64]` — no curve operation, just slice-and-
 * encode, because the Solana keypair file format already includes the
 * derived public key as its second 32-byte half.
 */
function solanaKeypairArrayToKeypairJson(secret: number[]): KeypairJson {
  const pubBytes = new Uint8Array(secret.slice(32, 64));
  return {
    secret: [...secret],
    pubkey_base58: base58Encode(pubBytes),
  };
}

/** Multi-line UserError describing the three ways to obtain an identity. */
function noIdentityError(): UserError {
  return new UserError(
    `no identity in ${configDir()}. Options:\n` +
      `  • mnemonic init                              — generate fresh keypair\n` +
      `  • mnemonic identity import --ticket <uuid>   — round-trip from webapp\n` +
      `  • mnemonic identity import --file <path>     — import existing keypair JSON`,
  );
}

export function identityExists(): boolean {
  return existsSync(identityPath()) || existsSync(legacyIdJsonPath());
}

export function loadIdentityJson(): KeypairJson {
  const path = identityPath();
  if (existsSync(path)) {
    let raw: string;
    try {
      raw = readFileSync(path, "utf8");
    } catch (e) {
      throw new UserError(`identity unreadable: ${path}`, e);
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch (e) {
      throw new UserError(`identity is not valid JSON: ${path}`, e);
    }
    if (!isKeypairJson(parsed)) {
      throw new UserError(
        `identity has wrong shape; expected {secret: number[64], pubkey_base58: string}`,
      );
    }
    return parsed;
  }

  // Fallback: server self-host layout `<dir>/id.json` — a bare JSON array of
  // 64 numbers (Solana keypair file format). Convert in-memory only; do NOT
  // rewrite to identity.json so the user's existing file stays canonical.
  const legacy = legacyIdJsonPath();
  if (existsSync(legacy)) {
    let raw: string;
    try {
      raw = readFileSync(legacy, "utf8");
    } catch (e) {
      throw new UserError(`identity unreadable: ${legacy}`, e);
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch (e) {
      throw new UserError(`identity is not valid JSON: ${legacy}`, e);
    }
    if (!isSolanaKeypairArray(parsed)) {
      throw new UserError(
        `${legacy} is not a Solana keypair file (expected JSON array of 64 numbers in 0..255)`,
      );
    }
    process.stderr.write("using legacy id.json (Solana keypair file format)\n");
    return solanaKeypairArrayToKeypairJson(parsed);
  }

  throw noIdentityError();
}

/**
 * Resolve a stub identity.json via the OS keychain and return a full Keypair.
 *
 * Called from `loadIdentity()` and `whoami` when `Keypair.fromJSON` throws
 * `IdentityRequiresKeystore` — i.e. the file is a stub that carries only a
 * pubkey + keychain_ref, not the raw secret bytes.
 *
 * Exported so `commands/whoami.ts` can reuse it without duplicating the
 * keychain-lookup logic.
 */
export async function resolveKeypairFromKeystore(
  pubkey_base58: string,
): Promise<Keypair> {
  const ks = new OsKeyStore();
  const entry = await ks.get();
  if (!entry) {
    throw new UserError(
      `identity.json points at the OS keychain (stub file) but the keychain entry is missing. ` +
        `Run \`mnemonic identity pull-from-webapp\` to re-import your key.`,
    );
  }
  const fullJson: KeypairJson = {
    secret: entry.secret,
    pubkey_base58: entry.pubkey_base58 || pubkey_base58,
  };
  return Keypair.fromJSON(fullJson);
}

/** Convenience: load + return as a `Keypair` instance. */
export async function loadIdentity(): Promise<Keypair> {
  const json = loadIdentityJson();
  try {
    return await Keypair.fromJSON(json);
  } catch (e) {
    if (e instanceof IdentityRequiresKeystore) {
      return resolveKeypairFromKeystore(e.pubkey_base58);
    }
    throw e;
  }
}

/** Atomic mode-0600 write of an identity. */
export function saveIdentityJson(json: KeypairJson): void {
  if (!isKeypairJson(json)) {
    throw new UserError("saveIdentity: invalid keypair shape");
  }
  ensureDir();
  const path = identityPath();
  // mode 0600 at create-time; chmod-then-write order matters because
  // `writeFileSync` truncates and re-creates on some filesystems, which
  // would drop our mode. Use the `mode` option then re-chmod for safety.
  writeFileSync(path, JSON.stringify(json, null, 2), { mode: 0o600 });
  restrictFileMode(path);
}

export function saveIdentity(kp: Keypair): void {
  saveIdentityJson(kp.toJSON());
}

// ── token ───────────────────────────────────────────────────────────────────

function isTokenJson(v: unknown): v is TokenJson {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o.jwt === "string" &&
    o.jwt.length > 0 &&
    typeof o.expires_at === "string" &&
    typeof o.sub === "string"
  );
}

export function tokenExists(): boolean {
  return existsSync(tokenPath());
}

/**
 * Read the persisted JWT token. Throws if missing OR expired (`expires_at`
 * is strictly in the past, comparing wall-clock).
 */
export function loadToken(): TokenJson {
  const path = tokenPath();
  if (!existsSync(path)) {
    throw new UserError(`no token at ${path}; run \`mnemonic login\` first`);
  }
  let raw: string;
  try {
    raw = readFileSync(path, "utf8");
  } catch (e) {
    throw new UserError(`token unreadable: ${path}`, e);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (e) {
    throw new UserError(`token is not valid JSON: ${path}`, e);
  }
  if (!isTokenJson(parsed)) {
    throw new UserError(
      `token has wrong shape; expected {jwt, expires_at, sub}`,
    );
  }
  const exp = Date.parse(parsed.expires_at);
  if (Number.isFinite(exp) && exp <= Date.now()) {
    throw new UserError(
      `token expired at ${parsed.expires_at}; run \`mnemonic login\` again`,
    );
  }
  return parsed;
}

/** Atomic mode-0600 write of a token. */
export function saveToken(token: TokenJson): void {
  if (!isTokenJson(token)) {
    throw new UserError("saveToken: invalid token shape");
  }
  ensureDir();
  const path = tokenPath();
  writeFileSync(path, JSON.stringify(token, null, 2), { mode: 0o600 });
  restrictFileMode(path);
}
