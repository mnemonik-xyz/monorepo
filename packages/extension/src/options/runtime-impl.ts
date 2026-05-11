// Production OptionsRuntime. Wires the typed facades from `runtime.ts`
// to the existing infra:
//
//   - `settings` → `src/settings.ts` (chrome.storage.local under
//     `settings.v1`).
//   - `auth` → STUB. T16 will replace with the real Google OAuth client
//     (`src/auth/google-oauth.ts`). Today the stub throws on `signIn`
//     so the Storage section's "Switch to Cloud" branch surfaces a
//     clear "sign-in not yet wired" error instead of silently failing.
//   - `keyEscrow` → STUB. T17 will replace with the real key-escrow
//     client. Today calls reject so the Security section can render the
//     "coming soon" banner without breaking the form.
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

const SESSION_KEY = "auth_session.v1";

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
      // T16 stub — see file header. The Storage section's confirmation
      // dialog surfaces this error in a toast.
      throw new Error("Google sign-in is not yet wired (T16 pending)");
    },
    async getSession(): Promise<AuthSession | null> {
      try {
        const stored = (await chrome.storage.local.get([
          SESSION_KEY,
        ])) as Record<string, unknown>;
        const raw = stored[SESSION_KEY];
        if (!raw || typeof raw !== "object") return null;
        const obj = raw as Record<string, unknown>;
        if (
          typeof obj.google_sub !== "string" ||
          typeof obj.email !== "string" ||
          typeof obj.jwt !== "string"
        ) {
          return null;
        }
        return {
          google_sub: obj.google_sub,
          email: obj.email,
          jwt: obj.jwt,
        };
      } catch {
        return null;
      }
    },
    async clearSession(): Promise<void> {
      try {
        await chrome.storage.local.remove(SESSION_KEY);
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
    async rotate(): Promise<void> {
      throw new Error("Key escrow is not yet wired (T17 pending)");
    },
    async delete(): Promise<void> {
      throw new Error("Key escrow is not yet wired (T17 pending)");
    },
    async hasBlob(): Promise<boolean> {
      return false;
    },
  };
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
      // T18 will GET /api/attestations?count=true (or equivalent) and
      // surface the real cloud-side row count for the active session.
      // Until then, return 0 so the Cloud→Local dialog renders a stable
      // placeholder rather than the misleading local count.
      return 0;
    },
    async enqueueAll(): Promise<void> {
      // T18 will iterate every row and push to `pending_uploads`. Today
      // this is a no-op — the SW alarm already has the cloud-sync drain
      // stub (`runtime/sync/cloud-client.ts`) that T18 will fill in.
      return;
    },
    subscribeProgress(cb: (e: MigrationProgressEvent) => void): () => void {
      // Single synthetic done event so the migration UI doesn't hang
      // waiting for progress that won't arrive until T18.
      const handle = setTimeout(() => {
        cb({ attempted: 0, flushed: 0, total: 0, done: true });
      }, 0);
      return () => clearTimeout(handle);
    },
    async exportAll(): Promise<Uint8Array> {
      throw new Error("Cloud → Local export is not yet wired (T18 pending)");
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
