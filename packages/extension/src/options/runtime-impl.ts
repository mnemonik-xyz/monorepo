// Production OptionsRuntime. Wires the typed facades from `runtime.ts`
// to the existing infra:
//
//   - `settings` → `src/settings.ts` (chrome.storage.local under
//     `settings.v1`).
//   - `auth` → T16 real implementation. `signIn` runs Google OAuth via
//     `src/auth/google-oauth.ts::signInWithGoogle` and persists the
//     result through `src/auth/session.ts::setSession`. `getSession`
//     reads through the SAME `getSession` helper the popup uses so
//     the persisted shape is unified (audit B3 / AUD-C-03).
//   - `keyEscrow` → T17 real implementation. `rotate` re-derives the
//     Argon2id wrap-key and PUTs a fresh blob; `delete` issues
//     `DELETE /api/key-escrow`; `hasBlob` probes via GET and treats
//     `EscrowNotFoundError` (or any transport hiccup) as "no blob".
//     All three run the T15 extension-bootstrap handshake first so
//     the wire-level Bearer is `aud=extension` (audit B1).
//   - `cloudSync` → T18 wiring. `enqueueAll` iterates every local row
//     and calls `IndexedDbStore.enqueue` so the SW alarm can drain
//     them; `subscribeProgress` polls the local queue size while the
//     drain runs. `countCloudAttestations` stays at 0 until a server
//     endpoint exists.
//   - `identity` → reads `chrome.storage.local.identity` (populated by
//     T16 onboarding). Export / import keypair are STUB — T05 owns the
//     WASM `export_keypair_json` / `import_keypair_json` exports the
//     real impl will call into.
//
// Heavy modules (WASM, IndexedDB) are lazy-loaded so the options page
// initial bundle stays small.

import { loadSettings, updateSettings } from "../settings.js";
import {
  deleteEscrow,
  fetchEscrow,
  rotatePassphrase,
  EscrowNotFoundError,
  bytesToBase64,
} from "../auth/key-escrow.js";
import { signInWithGoogle } from "../auth/google-oauth.js";
import {
  getSession,
  setSession,
  clearSession,
  jwtExpiresAtMs,
} from "../auth/session.js";
import { ensureExtensionJwt } from "../auth/extension-bootstrap.js";
import { IDENTITY_KEY, IDENTITY_SECRET_KEY } from "../auth/storage-keys.js";
import type { Session } from "../auth/types.js";
import type {
  AuthFacade,
  AuthSession,
  CloudSyncFacade,
  IdentityFacade,
  IdentitySnapshot,
  KeyEscrowFacade,
  MigrationProgressEvent,
  OptionsRuntime,
  PlainKeypairExport,
  SettingsFacade,
} from "./runtime.js";

/** T25 audit-mandated warning string. Lives in a top-level constant so
 *  the exporter cannot drift from the import-time documentation and so
 *  the test suite can assert the exact text without duplicating a long
 *  string literal. Security review insists this text accompanies every
 *  unencrypted-export download. */
export const PLAIN_EXPORT_WARNING =
  "Never share this file — anyone with it controls your identity. Store it in a password manager.";

/** chrome.storage.local keys the popup + options runtimes share for
 *  the `{pubkey_base58, created_at?}` + 64-byte secret pair. The
 *  canonical definitions now live in `src/auth/storage-keys.ts` —
 *  re-export here for callers that still depend on the old import
 *  path (PR136-C-01 / SA-T25-003). */
export { IDENTITY_KEY, IDENTITY_SECRET_KEY } from "../auth/storage-keys.js";

/** Lightweight base58 (Bitcoin alphabet) validator. The full base58
 *  decode is not needed here — we just sanity-check that every
 *  character of the imported pubkey is in the canonical alphabet so an
 *  obviously-malformed key (e.g. raw hex or accidentally pasted text)
 *  trips the validation at the UI seam rather than at the WASM
 *  boundary later. */
const BASE58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Pre-computed Set for O(1) character membership lookups (PR136-C-06).
 *  Pulled out of `isLikelyBase58` so the 58-entry Set is constructed
 *  once at module-load instead of `String.indexOf`-scanning the
 *  alphabet on every character. */
const BASE58_SET: Set<string> = new Set(BASE58_ALPHABET);

function isLikelyBase58(s: string): boolean {
  if (typeof s !== "string" || s.length === 0) return false;
  // Solana Ed25519 pubkeys are 32 bytes → ~43-44 base58 chars. We use a
  // generous 32..64 window so unusually short / long encodings still
  // reach the WASM validator for the authoritative check.
  if (s.length < 32 || s.length > 64) return false;
  for (let i = 0; i < s.length; i++) {
    if (!BASE58_SET.has(s[i]!)) return false;
  }
  return true;
}

/** Fallback session lifetime when neither the server response nor the
 *  JWT carries a usable expiry. Mirrors the popup's onboarding default
 *  so both writers stay in lock-step. */
const FALLBACK_SESSION_MS = 60 * 60 * 1000;

/** Project the popup's persisted `Session` shape onto the options-side
 *  `AuthSession` DTO. The session.ts module owns the on-disk schema;
 *  this function is a one-line shape mapper, not an alternative writer. */
function projectSession(s: Session): AuthSession {
  return {
    googleSub: s.googleSub,
    email: s.profile.email,
    jwt: s.jwt,
  };
}

/** Build the production OptionsRuntime. Called once from `main.tsx`. */
export function createDefaultOptionsRuntime(): OptionsRuntime {
  return {
    settings: createSettingsFacade(),
    auth: createAuthFacade(),
    identity: createIdentityFacade(),
    keyEscrow: createKeyEscrowFacade(),
    cloudSync: createCloudSyncFacade(),
    about: readAboutMeta(),
  };
}

function createSettingsFacade(): SettingsFacade {
  return {
    load: () => loadSettings(),
    update: (patch) => updateSettings(patch),
  };
}

function createAuthFacade(): AuthFacade {
  return {
    async signIn(): Promise<AuthSession> {
      // Audit B2 / AUD-C-02: T16 has shipped — wire signInWithGoogle
      // here, mirror what `popup/runtime.ts::auth.signIn` does, and
      // persist through the SAME `setSession` writer so the on-disk
      // shape is unified across realms (B3 / AUD-C-03).
      const result = await signInWithGoogle();
      const next: Session = {
        jwt: result.jwt,
        googleSub: result.googleSub,
        profile: result.profile,
        jwtExpiresAt:
          result.jwtExpiresAt > 0
            ? result.jwtExpiresAt
            : (jwtExpiresAtMs(result.jwt) ?? Date.now() + FALLBACK_SESSION_MS),
        signedInAt: Date.now(),
      };
      await setSession(next);
      return projectSession(next);
    },
    async getSession(): Promise<AuthSession | null> {
      const s = await getSession();
      if (!s) return null;
      return projectSession(s);
    },
    async clearSession(): Promise<void> {
      await clearSession();
    },
  };
}

/** SA-T25-002 TOCTOU serialiser. `generate()` runs
 *  read → check → WASM → write across multiple awaits; two concurrent
 *  invocations (e.g. a double-clicked button or two open Options tabs)
 *  could both pass the existence check before either completes the
 *  write. We funnel every call through this module-level lock so the
 *  second caller awaits the first and then re-reads storage — its
 *  existence check will trip and throw, surfacing the duplicate-click
 *  as a typed error in the UI rather than a silently overwritten key. */
let generateInFlight: Promise<IdentitySnapshot> | null = null;

function createIdentityFacade(): IdentityFacade {
  return {
    async load(): Promise<IdentitySnapshot | null> {
      try {
        const stored = (await chrome.storage.local.get([
          IDENTITY_KEY,
        ])) as Record<string, unknown>;
        const raw = stored[IDENTITY_KEY];
        if (!raw || typeof raw !== "object") return null;
        const obj = raw as Record<string, unknown>;
        const pub = obj.pubkey_base58;
        if (typeof pub !== "string" || pub.length === 0) return null;
        const created =
          typeof obj.created_at === "number" && Number.isFinite(obj.created_at)
            ? obj.created_at
            : undefined;
        const out: IdentitySnapshot = {
          pubkey_base58: pub,
          did: `did:sol:${pub}`,
          ...(created !== undefined ? { created_at: created } : {}),
        };
        return out;
      } catch {
        return null;
      }
    },
    async generate(): Promise<IdentitySnapshot> {
      // SA-T25-002: serialise concurrent generate() calls so the
      // read → check → write sequence is observed atomically. A second
      // caller awaits the first; after the first resolves, the second
      // re-runs the existence check and throws because the now-stored
      // keypair trips the clobber-guard. The lock is released in a
      // finally{} so a thrown WASM init failure does not wedge future
      // calls.
      if (generateInFlight) {
        await generateInFlight.catch(() => undefined);
      }
      const run = async (): Promise<IdentitySnapshot> => {
        // Refuse to clobber an existing identity. The UI confirms the
        // destructive regenerate path via a dialog and calls `clear()`
        // FIRST — by the time we land here the storage should be empty.
        const existing = (await chrome.storage.local.get([
          IDENTITY_KEY,
          IDENTITY_SECRET_KEY,
        ])) as Record<string, unknown>;
        if (
          existing[IDENTITY_KEY] !== undefined ||
          existing[IDENTITY_SECRET_KEY] !== undefined
        ) {
          throw new Error(
            "Refusing to overwrite existing identity — clear it first",
          );
        }
        // WASM `generate_keypair` uses `getrandom`'s `js` feature →
        // `crypto.getRandomValues`. CSPRNG-quality entropy. The base58
        // pubkey is encoded inside the WASM boundary so we don't have to
        // roll one here.
        const { generateKeypair } = await import("../runtime/sign/cose.js");
        const kp = await generateKeypair();
        // SA-T25-004: mirror the byte-range normalisation that
        // importPlain / exportPlain perform so all three storage-write
        // paths share the same invariant. `generateKeypair` already
        // validates internally — this is defence-in-depth against a
        // future binding swap that bypasses the cose.ts normalisation.
        if (!Array.isArray(kp.secret) || kp.secret.length !== 64) {
          throw new Error("generate: WASM returned a malformed secret");
        }
        const normalisedSecret: number[] = [];
        for (const n of kp.secret) {
          const num = typeof n === "number" ? n : Number(n);
          if (!Number.isInteger(num) || num < 0 || num > 255) {
            throw new Error("generate: WASM returned a non-byte value");
          }
          normalisedSecret.push(num);
        }
        const created_at = Date.now();
        await chrome.storage.local.set({
          [IDENTITY_KEY]: { pubkey_base58: kp.pubkey_base58, created_at },
          [IDENTITY_SECRET_KEY]: normalisedSecret,
        });
        return {
          pubkey_base58: kp.pubkey_base58,
          did: `did:sol:${kp.pubkey_base58}`,
          created_at,
        };
      };
      generateInFlight = run();
      try {
        return await generateInFlight;
      } finally {
        generateInFlight = null;
      }
    },
    async clear(): Promise<void> {
      // Best-effort: chrome.storage.local.remove is idempotent for
      // missing keys. We never log the secret on the way out.
      await chrome.storage.local.remove([IDENTITY_KEY, IDENTITY_SECRET_KEY]);
    },
    async exportEncrypted(): Promise<Uint8Array> {
      // T05 follow-up: WASM `export_keypair_json` + passphrase wrap.
      throw new Error("Export keypair is not yet wired (T05 follow-up)");
    },
    async importEncrypted(): Promise<IdentitySnapshot> {
      // T05 follow-up: WASM `import_keypair_json` + passphrase unwrap.
      throw new Error("Import keypair is not yet wired (T05 follow-up)");
    },
    async exportPlain(): Promise<PlainKeypairExport> {
      const stored = (await chrome.storage.local.get([
        IDENTITY_KEY,
        IDENTITY_SECRET_KEY,
      ])) as Record<string, unknown>;
      const raw = stored[IDENTITY_KEY];
      const secret = stored[IDENTITY_SECRET_KEY];
      if (!raw || typeof raw !== "object") {
        throw new Error("No identity to export");
      }
      const obj = raw as Record<string, unknown>;
      const pub = obj.pubkey_base58;
      if (typeof pub !== "string" || pub.length === 0) {
        throw new Error("No identity to export");
      }
      if (!Array.isArray(secret) || secret.length !== 64) {
        throw new Error(
          "Identity secret is missing or malformed — refusing to export",
        );
      }
      // Defensive byte-range check so the on-disk JSON cannot embed a
      // value > 255 (would be a structured-clone bug we surface early).
      const normalised: number[] = [];
      for (const n of secret) {
        const num = typeof n === "number" ? n : Number(n);
        if (!Number.isInteger(num) || num < 0 || num > 255) {
          throw new Error("Identity secret contains non-byte value");
        }
        normalised.push(num);
      }
      return {
        version: 1,
        pubkey_base58: pub,
        secret: normalised,
        exported_at: new Date().toISOString(),
        warning: PLAIN_EXPORT_WARNING,
      };
    },
    async importPlain(payload: unknown): Promise<IdentitySnapshot> {
      // Validate the envelope BEFORE writing anything — the toast on
      // failure surfaces the specific Error.message verbatim so the
      // user knows which field is wrong.
      if (!payload || typeof payload !== "object") {
        throw new Error("Wrong file format: expected a JSON object");
      }
      const obj = payload as Record<string, unknown>;
      if (obj.version !== 1) {
        throw new Error(
          `Wrong file format: unsupported version ${String(obj.version)} (expected 1)`,
        );
      }
      const pub = obj.pubkey_base58;
      if (typeof pub !== "string") {
        throw new Error("Invalid public key: expected a base58 string");
      }
      if (!isLikelyBase58(pub)) {
        throw new Error(
          "Invalid public key: not a recognisable base58 Ed25519 pubkey",
        );
      }
      const secret = obj.secret;
      if (!Array.isArray(secret)) {
        throw new Error("Wrong secret length: expected an array of 64 bytes");
      }
      if (secret.length !== 64) {
        throw new Error(
          `Wrong secret length: got ${secret.length} bytes, expected 64`,
        );
      }
      // Byte-range validation — keeps a hand-edited JSON with > 255
      // entries from poisoning chrome.storage. PR136-C-04: the message
      // says "byte value" (the actual failure mode), not "secret length"
      // — by this point the 64-element length is already confirmed.
      const normalised: number[] = [];
      for (let i = 0; i < secret.length; i++) {
        const n = secret[i];
        const num = typeof n === "number" ? n : Number(n);
        if (!Number.isInteger(num) || num < 0 || num > 255) {
          throw new Error(
            `Invalid secret byte at index ${i}: expected an integer in [0, 255]`,
          );
        }
        normalised.push(num);
      }
      // PR136-C-02: forward `exported_at` from the import envelope as
      // `created_at` when it parses to a sensible wall-clock time, so a
      // round-trip (export → import) preserves the original age. The
      // exporter writes ISO-8601 strings; we accept either ISO strings
      // or epoch-ms numbers for backward compatibility. Guard against:
      //   - missing / wrong-type values (fall back to Date.now()),
      //   - unparseable strings (NaN getTime()),
      //   - non-positive timestamps,
      //   - absurdly-future timestamps (clock skew tolerance: 24h).
      const now = Date.now();
      const FUTURE_TOLERANCE_MS = 24 * 60 * 60 * 1000;
      let created_at = now;
      const exportedAt = obj.exported_at;
      if (typeof exportedAt === "string" || typeof exportedAt === "number") {
        const parsed =
          typeof exportedAt === "number"
            ? exportedAt
            : new Date(exportedAt).getTime();
        if (
          Number.isFinite(parsed) &&
          parsed > 0 &&
          parsed <= now + FUTURE_TOLERANCE_MS
        ) {
          created_at = parsed;
        }
      }
      await chrome.storage.local.set({
        [IDENTITY_KEY]: { pubkey_base58: pub, created_at },
        [IDENTITY_SECRET_KEY]: normalised,
      });
      return {
        pubkey_base58: pub,
        did: `did:sol:${pub}`,
        created_at,
      };
    },
  };
}

function createKeyEscrowFacade(): KeyEscrowFacade {
  return {
    async rotate(oldPassphrase: string, nextPassphrase: string): Promise<void> {
      // Production wiring (T17): forward to the real key-escrow client.
      // The Security section's TDD anchor `rotate_passphrase_re_encrypts_blob`
      // already mocks this seam directly; in production we forward the
      // typed errors as-is so the Security toast surfaces the right copy
      // (wrong-passphrase, 429 rate-limit, etc.).
      const extJwt = await readExtensionJwtOrThrow("rotate");
      await rotatePassphrase(extJwt, oldPassphrase, nextPassphrase);
    },
    async delete(): Promise<void> {
      const extJwt = await readExtensionJwtOrThrow("delete");
      await deleteEscrow(extJwt);
    },
    async hasBlob(): Promise<boolean> {
      try {
        const extJwt = await readExtensionJwt();
        if (!extJwt) return false;
        await fetchEscrow(extJwt);
        return true;
      } catch (err) {
        if (err instanceof EscrowNotFoundError) return false;
        // Any other error (network / auth / server) leaves the question
        // open — surface as "no blob known" so the Security UI does not
        // wedge into the false-positive state where Delete is enabled
        // against a server that just had a hiccup.
        return false;
      }
    },
  };
}

/** Lift the cached Google session via `getSession()` (single source of
 *  truth — B3 / AUD-C-03), then run the T15 extension-bootstrap
 *  handshake to redeem an `aud=extension` JWT for /api/key-escrow*
 *  (audit B1 / AUD-S-01). Returns `null` when no session is cached so
 *  callers can branch defensively (e.g. `hasBlob` returns false). */
async function readExtensionJwt(): Promise<string | null> {
  const session = await getSession();
  if (!session) return null;
  try {
    return await ensureExtensionJwt(session.jwt);
  } catch {
    // Bootstrap failures bubble up at the call site as "no blob known"
    // for `hasBlob`; the throw variant surfaces them as toast errors.
    return null;
  }
}

async function readExtensionJwtOrThrow(where: string): Promise<string> {
  const session = await getSession();
  if (!session) {
    throw new Error(
      `${where}: no Google session cached — sign in before managing escrow`,
    );
  }
  return await ensureExtensionJwt(session.jwt);
}

function createCloudSyncFacade(): CloudSyncFacade {
  return {
    async countLocalAttestations(): Promise<number> {
      // Lazy-import the IndexedDB store so the options page first-paint
      // bundle stays small — only the Storage section migration flow
      // pays the cost.
      try {
        const stored = (await chrome.storage.local.get([
          IDENTITY_KEY,
        ])) as Record<string, unknown>;
        const identity = stored[IDENTITY_KEY] as
          | { pubkey_base58?: string }
          | undefined;
        if (!identity?.pubkey_base58) return 0;
        const { IndexedDbStore } =
          await import("../runtime/store/indexeddb.js");
        const store = new IndexedDbStore();
        return await store.count(identity.pubkey_base58);
      } catch {
        return 0;
      }
    },
    async countCloudAttestations(): Promise<number> {
      // No `/api/attestations?count=true` endpoint exists today; we keep
      // the stub returning 0 so the Cloud→Local dialog renders a stable
      // placeholder rather than the misleading local count. A follow-up
      // backlog item will add the server endpoint.
      return 0;
    },
    async enqueueAll(): Promise<void> {
      // Audit B2 / AUD-C-02: T18 has shipped — iterate every local row
      // for the active identity and call `IndexedDbStore.enqueue` so
      // the SW alarm-driven `drainPendingUploads` (`background/
      // cloud-sync.ts`) flushes them on the next tick.
      try {
        const stored = (await chrome.storage.local.get([
          IDENTITY_KEY,
        ])) as Record<string, unknown>;
        const identity = stored[IDENTITY_KEY] as
          | { pubkey_base58?: string }
          | undefined;
        if (!identity?.pubkey_base58) return;
        const { IndexedDbStore } =
          await import("../runtime/store/indexeddb.js");
        const store = new IndexedDbStore();
        const rows = await store.allByOwner(identity.pubkey_base58);
        for (const row of rows) {
          await store.enqueue(row.attestation_id);
        }
      } catch (err) {
        // Bubble up so the Storage section can render the migration-
        // failed toast. The dialog stays open via the error path in
        // Storage.tsx's onConfirmToCloud catch.
        throw err instanceof Error
          ? err
          : new Error(`enqueueAll failed: ${String(err)}`);
      }
    },
    subscribeProgress(cb: (e: MigrationProgressEvent) => void): () => void {
      // Audit B2: surface live queue drain progress. We poll
      // `pending_uploads` size every 2s; the SW alarm-tick drains the
      // queue server-side. Done when the queue is empty.
      let cancelled = false;
      let pollHandle: ReturnType<typeof setTimeout> | null = null;
      let total = 0;
      let started = false;
      const poll = async (): Promise<void> => {
        if (cancelled) return;
        try {
          const { IndexedDbStore } =
            await import("../runtime/store/indexeddb.js");
          const store = new IndexedDbStore();
          const pending = await store.pendingCount();
          if (!started) {
            total = pending;
            started = true;
          }
          const flushed = Math.max(0, total - pending);
          const event: MigrationProgressEvent = {
            attempted: total,
            flushed,
            total,
            ...(pending === 0 ? { done: true } : {}),
          };
          cb(event);
          if (pending > 0 && !cancelled) {
            pollHandle = setTimeout(() => void poll(), 2000);
          }
        } catch (err) {
          if (!cancelled) {
            cb({
              attempted: 0,
              flushed: 0,
              total: 0,
              done: true,
              error: err instanceof Error ? err.message : String(err),
            });
          }
        }
      };
      // First tick immediately so callers see at least one event.
      pollHandle = setTimeout(() => void poll(), 0);
      return () => {
        cancelled = true;
        if (pollHandle !== null) clearTimeout(pollHandle);
      };
    },
    async exportAll(): Promise<Uint8Array> {
      // Cloud → Local archive: bundle every local row as JSON. The COSE
      // bytes are emitted as base64 so the archive is JSON-clean. The
      // Storage section triggers a download of the returned bytes.
      const stored = (await chrome.storage.local.get([IDENTITY_KEY])) as Record<
        string,
        unknown
      >;
      const identity = stored[IDENTITY_KEY] as
        | { pubkey_base58?: string }
        | undefined;
      if (!identity?.pubkey_base58) {
        // Nothing to export; emit an empty-but-valid bundle so the
        // download still goes through and the UI doesn't error out.
        return new TextEncoder().encode(JSON.stringify({ rows: [] }, null, 2));
      }
      const { IndexedDbStore } = await import("../runtime/store/indexeddb.js");
      const store = new IndexedDbStore();
      const rows = await store.allByOwner(identity.pubkey_base58);
      const serialised = rows.map((r) => ({
        ...r,
        cose_bytes: bytesToBase64(r.cose_bytes),
        embedding: Array.from(r.embedding),
      }));
      return new TextEncoder().encode(
        JSON.stringify({ rows: serialised }, null, 2),
      );
    },
  };
}

function readAboutMeta(): { version: string; buildHash?: string } {
  try {
    const m = chrome.runtime.getManifest();
    return { version: m.version ?? "0.0.0" };
  } catch {
    return { version: "0.0.0" };
  }
}
