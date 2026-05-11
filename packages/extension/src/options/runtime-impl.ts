// Production OptionsRuntime. Wires the typed facades from `runtime.ts`
// to the existing infra:
//
//   - `settings` → `src/settings.ts` (chrome.storage.local under
//     `settings.v1`).
//   - `auth` → STUB. T16 will replace with the real Google OAuth client
//     (`src/auth/google-oauth.ts`). Today the stub throws on `signIn`
//     so the Storage section's "Switch to Cloud" branch surfaces a
//     clear "sign-in not yet wired" error instead of silently failing.
//   - `keyEscrow` → T17 real implementation. `rotate` re-derives the
//     Argon2id wrap-key and PUTs a fresh blob; `delete` issues
//     `DELETE /api/key-escrow`; `hasBlob` probes via GET and treats
//     `EscrowNotFoundError` (or any transport hiccup) as "no blob".
//   - `cloudSync` → STUB. T18 will replace with deferred-signing client.
//     Today `enqueueAll` is a no-op and `subscribeProgress` emits a
//     single `done: true` event so the migration UI doesn't hang.
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
} from "../auth/key-escrow.js";
import { getSession, clearSession } from "../auth/session.js";
import { getOrRefreshExtensionJwt } from "../auth/bootstrap-ticket.js";
import type {
  AuthFacade,
  AuthSession,
  CloudSyncFacade,
  IdentityFacade,
  IdentitySnapshot,
  KeyEscrowFacade,
  MigrationProgressEvent,
  OptionsRuntime,
  SettingsFacade,
} from "./runtime.js";

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
      // Production: dynamic-import the same google-oauth flow the popup
      // uses, persist the resulting session via auth/session.ts (the
      // single source of truth for the `session.v1` slot), and project
      // to the options-side AuthSession shape.
      const { signInWithGoogle } = await import("../auth/google-oauth.js");
      const { setSession } = await import("../auth/session.js");
      const result = await signInWithGoogle();
      const now = Date.now();
      await setSession({
        jwt: result.jwt,
        googleSub: result.googleSub,
        profile: result.profile,
        jwtExpiresAt: result.jwtExpiresAt,
        signedInAt: now,
      });
      return {
        google_sub: result.googleSub,
        email: result.profile.email,
        jwt: result.jwt,
      };
    },
    async getSession(): Promise<AuthSession | null> {
      const session = await getSession();
      if (!session) return null;
      return {
        google_sub: session.googleSub,
        email: session.profile.email,
        jwt: session.jwt,
      };
    },
    async clearSession(): Promise<void> {
      try {
        await clearSession();
      } catch {
        // Best-effort.
      }
    },
  };
}

function createIdentityFacade(): IdentityFacade {
  return {
    async load(): Promise<IdentitySnapshot | null> {
      try {
        const stored = (await chrome.storage.local.get(["identity"])) as Record<
          string,
          unknown
        >;
        const raw = stored.identity;
        if (!raw || typeof raw !== "object") return null;
        const obj = raw as Record<string, unknown>;
        const pub = obj.pubkey_base58;
        if (typeof pub !== "string" || pub.length === 0) return null;
        return { pubkey_base58: pub, did: `did:sol:${pub}` };
      } catch {
        return null;
      }
    },
    async exportEncrypted(): Promise<Uint8Array> {
      // T05 follow-up: WASM `export_keypair_json` + passphrase wrap.
      throw new Error("Export keypair is not yet wired (T05 follow-up)");
    },
    async importEncrypted(): Promise<IdentitySnapshot> {
      // T05 follow-up: WASM `import_keypair_json` + passphrase unwrap.
      throw new Error("Import keypair is not yet wired (T05 follow-up)");
    },
  };
}

function createKeyEscrowFacade(): KeyEscrowFacade {
  return {
    async rotate(oldPassphrase: string, nextPassphrase: string): Promise<void> {
      // FW-S-01 fix: key-escrow endpoints require aud=extension JWT, so
      // drive the bootstrap-ticket dance and pass the resulting token.
      const jwt = await readExtensionJwtOrThrow("rotate");
      await rotatePassphrase(jwt, oldPassphrase, nextPassphrase);
    },
    async delete(): Promise<void> {
      const jwt = await readExtensionJwtOrThrow("delete");
      await deleteEscrow(jwt);
    },
    async hasBlob(): Promise<boolean> {
      try {
        const jwt = await readExtensionJwt();
        if (!jwt) return false;
        await fetchEscrow(jwt);
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

/** Drive the aud=mcp → aud=extension bootstrap-ticket dance for the
 *  cached session. Returns the (fresh) aud=extension JWT or `null` when
 *  no session is present. Bubbles errors from the bootstrap handshake
 *  so the caller can surface them in a toast. */
async function readExtensionJwt(): Promise<string | null> {
  const session = await getSession();
  if (!session) return null;
  return getOrRefreshExtensionJwt(session);
}

async function readExtensionJwtOrThrow(where: string): Promise<string> {
  const jwt = await readExtensionJwt();
  if (!jwt) {
    throw new Error(
      `${where}: no Google session cached — sign in before managing escrow`,
    );
  }
  return jwt;
}

function createCloudSyncFacade(): CloudSyncFacade {
  return {
    async countLocalAttestations(): Promise<number> {
      // Lazy-import the IndexedDB store so the options page first-paint
      // bundle stays small — only the Storage section migration flow
      // pays the cost.
      try {
        const stored = (await chrome.storage.local.get(["identity"])) as Record<
          string,
          unknown
        >;
        const identity = stored.identity as
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
      // No dedicated server-side count endpoint yet (the MCP recall
      // returns top-K hits, not a total). Probe with a generic recall
      // and surface the cardinality as a soft lower-bound. Returns 0
      // when no session is bound. The Cloud→Local dialog frames this
      // as "approximately N rows" to set expectations.
      try {
        const session = await getSession();
        if (!session) return 0;
        const { CloudClient } = await import("../runtime/sync/cloud-client.js");
        const client = new CloudClient({ jwt: session.jwt });
        const hits = await client.recallRemote("memory", 1000);
        return hits.length;
      } catch {
        return 0;
      }
    },
    async enqueueAll(): Promise<void> {
      // Iterate every local row for the active identity and enqueue
      // each one. The SW alarm drain (T18) picks them up and drives
      // the deferred-signing flow.
      try {
        const stored = (await chrome.storage.local.get(["identity"])) as Record<
          string,
          unknown
        >;
        const identity = stored.identity as
          | { pubkey_base58?: string }
          | undefined;
        if (!identity?.pubkey_base58) return;
        const { IndexedDbStore } =
          await import("../runtime/store/indexeddb.js");
        const store = new IndexedDbStore();
        const rows = await store.listByOwner(identity.pubkey_base58, 10_000, 0);
        for (const row of rows) {
          try {
            await store.enqueue(row.attestation_id);
          } catch {
            // Best-effort; continue with the rest.
          }
        }
      } catch {
        // Best-effort migration trigger; the SW alarm still runs on
        // its own cadence.
      }
    },
    subscribeProgress(cb: (e: MigrationProgressEvent) => void): () => void {
      // Poll the pending_uploads count + recall row total so the UI
      // surfaces real progress as the alarm-drain dequeues rows. We
      // can't drive a true push-channel from inside the options page
      // (the SW would need to broadcast), so a 1.5s poll is sufficient.
      let cancelled = false;
      let attempted = 0;
      let total = 0;
      const tick = async (): Promise<void> => {
        if (cancelled) return;
        try {
          const { IndexedDbStore } =
            await import("../runtime/store/indexeddb.js");
          const store = new IndexedDbStore();
          const pending = await store.listPending();
          if (total === 0) {
            total = pending.length + attempted;
          }
          const flushed = total - pending.length;
          attempted = Math.max(attempted, flushed);
          cb({
            attempted,
            flushed,
            total,
            done: pending.length === 0,
          });
          if (pending.length === 0) return;
        } catch (err) {
          cb({
            attempted,
            flushed: 0,
            total,
            done: true,
            error: err instanceof Error ? err.message : String(err),
          });
          return;
        }
        setTimeout(() => void tick(), 1500);
      };
      setTimeout(() => void tick(), 0);
      return () => {
        cancelled = true;
      };
    },
    async exportAll(): Promise<Uint8Array> {
      // Read every local attestation row + COSE bytes for the active
      // identity and serialise to a single newline-delimited JSON
      // bundle. We deliberately avoid pulling in a zip dep — the .ndjson
      // bundle is small, self-describing, and easy to reconstruct.
      try {
        const stored = (await chrome.storage.local.get(["identity"])) as Record<
          string,
          unknown
        >;
        const identity = stored.identity as
          | { pubkey_base58?: string }
          | undefined;
        if (!identity?.pubkey_base58) {
          return new TextEncoder().encode("");
        }
        const { IndexedDbStore } =
          await import("../runtime/store/indexeddb.js");
        const store = new IndexedDbStore();
        const rows = await store.listByOwner(identity.pubkey_base58, 10_000, 0);
        const lines: string[] = [];
        for (const row of rows) {
          const cose = row.cose_bytes ? bytesToBase64(row.cose_bytes) : "";
          lines.push(
            JSON.stringify({
              attestation_id: row.attestation_id,
              content: row.content,
              tags: row.tags,
              source_meta: row.source_meta ?? null,
              content_hash: row.content_hash,
              signer_pubkey: row.signer_pubkey,
              solana_tx: row.solana_tx,
              arweave_tx: row.arweave_tx,
              created_at: row.created_at,
              cose_bytes_b64: cose,
            }),
          );
        }
        return new TextEncoder().encode(lines.join("\n"));
      } catch (err) {
        throw new Error(
          `Cloud → Local export failed: ${
            err instanceof Error ? err.message : String(err)
          }`,
        );
      }
    },
  };
}

function bytesToBase64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i] as number);
  }
  return btoa(bin);
}

function readAboutMeta(): { version: string; buildHash?: string } {
  try {
    const m = chrome.runtime.getManifest();
    return { version: m.version ?? "0.0.0" };
  } catch {
    return { version: "0.0.0" };
  }
}
