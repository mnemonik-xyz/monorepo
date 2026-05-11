// Persisted-session store. Lives on top of `chrome.storage.local` so the
// JWT + Google profile survive popup close / service-worker restart.
// Forward-incompatible changes to the `Session` shape MUST bump the
// storage key (currently `session.v1`) so a half-upgraded extension
// does not crash on read.
//
// Phase 1 prompts re-login on JWT expiry — refresh-token support is
// in the backlog (Decision 5 mentions Phase 2 refresh flow). The
// `getSession` reader returns `null` for an expired token so callers
// can branch on freshness without re-parsing the JWT.
//
// THREAT MODEL — chrome.storage.local is NOT encrypted at rest.
//
// On Chrome desktop the backing store is a per-extension sqlite file under
// `~/.../Local Extension Settings/<id>/`. Any process with filesystem
// access to that path can read the persisted JWT in plaintext. This is
// the chrome-extension standard: extensions do not have access to
// CryptoKey-backed secret storage or OS keychain APIs.
//
// The accepted threat model for Phase 1 (per user-spec § Cloud-tier
// privacy and § Restore-flow):
//   - in-scope:  protecting the user-recovery passphrase (handled by T17
//                Argon2id+AES-GCM, NOT stored in chrome.storage.local;
//                the wrapped blob lives server-side in `key_escrow_blobs`),
//                rate-limiting blob fetches server-side (5/24h/google_sub),
//                MV3 CSP (no eval / no remote scripts).
//   - out of scope: filesystem-level compromise of the user's Chrome
//                   profile (an attacker with that level of access can
//                   already exfiltrate the entire profile, including
//                   cookies for every site).
//
// If the JWT is leaked, the attacker gains read access to the user's
// memories (existing or future) until the JWT expires AND until the user
// rotates their Ed25519 keypair via the recovery passphrase. Mitigation
// is to limit JWT lifetime (server-issued, default 1h per T14).

import { SESSION_STORAGE_KEY, type Session } from "./types.js";

// ── chrome.storage shim ────────────────────────────────────────────────

/**
 * Minimal subset of `chrome.storage.local` this module needs. Lets
 * tests inject an in-memory shim without monkey-patching the global
 * `chrome` object. Production callers leave the parameter undefined
 * and the runtime resolves to `chrome.storage.local`.
 *
 * The chrome typings allow a default-values record on `get`
 * (`{key: defaultValue}` overload), but no caller in this codebase
 * uses it. The narrowed signature here keeps call-sites unambiguous;
 * widen later if a caller actually needs the defaults overload.
 */
export interface StorageArea {
  get: (keys: string | string[]) => Promise<Record<string, unknown>>;
  set: (items: Record<string, unknown>) => Promise<void>;
  remove: (keys: string | string[]) => Promise<void>;
}

function defaultStorage(): StorageArea {
  const c = (globalThis as { chrome?: typeof chrome }).chrome;
  if (!c?.storage?.local) {
    throw new Error("chrome.storage.local is unavailable in this context");
  }
  return c.storage.local as unknown as StorageArea;
}

// ── Read / write / clear ───────────────────────────────────────────────

/**
 * Read the persisted session. Returns `null` when:
 *
 *   - no session has been written yet,
 *   - the stored blob fails shape validation (e.g. partial write,
 *     forward-incompatible schema),
 *   - the JWT has expired (`jwtExpiresAt <= Date.now()`).
 *
 * Audit S7 / FW-S-09: when a session IS found but is expired, the row
 * is removed from storage before returning null. Previously the
 * expired row lingered forever (Phase 2 concern), but a stale JWT
 * sitting in chrome.storage.local widens the exfil window with no
 * upside — `clearSession` is cheap and the next sign-in overwrites
 * the row in either case.
 */
export async function getSession(
  storage: StorageArea = defaultStorage(),
  now: () => number = Date.now,
): Promise<Session | null> {
  let result: Record<string, unknown>;
  try {
    result = await storage.get(SESSION_STORAGE_KEY);
  } catch {
    return null;
  }
  const raw = result[SESSION_STORAGE_KEY];
  if (!isPersistedSession(raw)) return null;
  if (raw.jwtExpiresAt <= now()) {
    // Best-effort cleanup: never let a remove() failure surface as a
    // null return — the in-memory check above already guarantees the
    // caller sees null.
    try {
      await clearSession(storage);
    } catch {
      // swallow — caller still gets null.
    }
    return null;
  }
  return raw;
}

/**
 * Persist a session. Overwrites any previously-stored session. Caller
 * is responsible for computing `jwtExpiresAt` from the JWT `exp` claim
 * (helpers below) and for setting `signedInAt`.
 */
export async function setSession(
  session: Session,
  storage: StorageArea = defaultStorage(),
): Promise<void> {
  if (!isPersistedSession(session)) {
    throw new Error("session: refusing to persist malformed shape");
  }
  await storage.set({ [SESSION_STORAGE_KEY]: session });
}

/**
 * Clear the persisted session. No-op when no session is present (the
 * underlying `storage.remove` is idempotent).
 */
export async function clearSession(
  storage: StorageArea = defaultStorage(),
): Promise<void> {
  await storage.remove(SESSION_STORAGE_KEY);
}

// ── JWT exp helper ─────────────────────────────────────────────────────

/**
 * Extract the JWT's `exp` claim (in seconds) and return it as
 * milliseconds. Returns `null` when:
 *
 *   - the token does not have three dot-separated segments,
 *   - the payload is not valid base64url-encoded JSON,
 *   - the `exp` claim is missing or non-numeric.
 *
 * No signature verification — that's the server's responsibility.
 * Caller uses the result to populate `Session.jwtExpiresAt`.
 */
export function jwtExpiresAtMs(jwt: string): number | null {
  if (typeof jwt !== "string") return null;
  const parts = jwt.split(".");
  if (parts.length !== 3) return null;
  const payloadRaw = parts[1];
  if (payloadRaw === undefined || payloadRaw.length === 0) return null;
  const padded =
    payloadRaw.replace(/-/gu, "+").replace(/_/gu, "/") +
    "===".slice((payloadRaw.length + 3) % 4);
  let json: unknown;
  try {
    const bin = atob(padded);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    json = JSON.parse(new TextDecoder("utf-8").decode(bytes));
  } catch {
    return null;
  }
  if (!json || typeof json !== "object") return null;
  const exp = (json as Record<string, unknown>).exp;
  if (typeof exp !== "number" || !Number.isFinite(exp)) return null;
  return Math.floor(exp * 1000);
}

// ── shape guard ────────────────────────────────────────────────────────

function isPersistedSession(value: unknown): value is Session {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  if (typeof v.jwt !== "string" || v.jwt.length === 0) return false;
  if (typeof v.googleSub !== "string" || v.googleSub.length === 0) return false;
  if (typeof v.jwtExpiresAt !== "number" || !Number.isFinite(v.jwtExpiresAt)) {
    return false;
  }
  if (typeof v.signedInAt !== "number" || !Number.isFinite(v.signedInAt)) {
    return false;
  }
  const profile = v.profile;
  if (!profile || typeof profile !== "object") return false;
  const p = profile as Record<string, unknown>;
  if (typeof p.sub !== "string") return false;
  if (typeof p.email !== "string") return false;
  if (typeof p.email_verified !== "boolean") return false;
  if (typeof p.name !== "string") return false;
  if (typeof p.picture !== "string") return false;
  return true;
}
