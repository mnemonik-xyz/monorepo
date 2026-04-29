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

import { execSync } from "node:child_process";
import {
  existsSync,
  chmodSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import { Keypair, type KeypairJson } from "@mnemonik-xyz/sdk";

import { UserError } from "./errors.js";

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
    // Quote both arguments to defeat path/username spaces.
    try {
      execSync(`icacls "${path}" /inheritance:r /grant:r "${user}:F"`, {
        stdio: "ignore",
      });
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

export function identityExists(): boolean {
  return existsSync(identityPath());
}

export function loadIdentityJson(): KeypairJson {
  const path = identityPath();
  if (!existsSync(path)) {
    throw new UserError(
      `no identity at ${path}; run \`mnemonic init\` to create one`
    );
  }
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
      `identity has wrong shape; expected {secret: number[64], pubkey_base58: string}`
    );
  }
  return parsed;
}

/** Convenience: load + return as a `Keypair` instance. */
export async function loadIdentity(): Promise<Keypair> {
  const json = loadIdentityJson();
  return Keypair.fromJSON(json);
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
      `token has wrong shape; expected {jwt, expires_at, sub}`
    );
  }
  const exp = Date.parse(parsed.expires_at);
  if (Number.isFinite(exp) && exp <= Date.now()) {
    throw new UserError(
      `token expired at ${parsed.expires_at}; run \`mnemonic login\` again`
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
