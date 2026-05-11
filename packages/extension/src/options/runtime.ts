// Options-page runtime facade. Mirrors the popup's `runtime.ts`
// pattern: the page consumes a single typed interface, the production
// implementation lives in `runtime-impl.ts`, and tests inject stubs via
// `setOptionsRuntime`. This keeps T16 (Google sign-in) and T17 (key-
// escrow client) integration points behind a stable seam — both are
// stubs today; they fulfil the same interface when they land.
//
// T18 (cloud-tier sync) is similarly stubbed under `cloudSync` so the
// Storage section's "Switch to Cloud" migration progress UI has a real
// callback shape to render against.

import type { SettingsV1 } from "../settings.js";

// ── Identity / auth (T16 will fulfil) ───────────────────────────────────────

export interface AuthSession {
  /** Stable Google subject id — the same value the server uses to key
   *  `google_identity_links` and `key_escrow_blobs`. */
  google_sub: string;
  /** Display label (e.g. `name@gmail.com`). Used by the UI only. */
  email: string;
  /** Server-issued JWT (`aud=mcp`) bound to this Google sub. The
   *  options page never inspects the JWT contents — it just hands the
   *  string back to the runtime. */
  jwt: string;
}

export interface AuthFacade {
  /** Launch the Google OAuth flow via `chrome.identity.launchWebAuthFlow`.
   *  Resolves with the freshly-minted session, OR throws on user-cancel
   *  / network error. T16 will replace this stub with the real impl. */
  signIn(): Promise<AuthSession>;
  /** Returns the active session if one is cached, otherwise null. */
  getSession(): Promise<AuthSession | null>;
  /** Clear the cached session + revoke the server token. */
  clearSession(): Promise<void>;
}

// ── Identity / keypair (T05 owns the WASM exports; T16 wires storage) ──────

export interface IdentitySnapshot {
  pubkey_base58: string;
  /** `did:sol:<pubkey>` — derived from `pubkey_base58`. */
  did: string;
}

export interface IdentityFacade {
  load(): Promise<IdentitySnapshot | null>;
  /** Export the in-storage keypair as a passphrase-encrypted JSON blob.
   *  Returns the blob bytes; the UI offers it as a download. */
  exportEncrypted(passphrase: string): Promise<Uint8Array>;
  /** Inverse of `exportEncrypted`. Throws on wrong passphrase / tampered
   *  blob; the options page shows the error in a toast. */
  importEncrypted(
    blob: Uint8Array,
    passphrase: string,
  ): Promise<IdentitySnapshot>;
}

// ── Key escrow (T17 will fulfil) ────────────────────────────────────────────

export interface KeyEscrowFacade {
  /** Re-derive the wrap-key from `nextPassphrase`, re-encrypt the
   *  Ed25519 secret, and `PUT /api/key-escrow`. */
  rotate(oldPassphrase: string, nextPassphrase: string): Promise<void>;
  /** `DELETE /api/key-escrow`. */
  delete(): Promise<void>;
  /** Returns whether a blob exists server-side for the active session. */
  hasBlob(): Promise<boolean>;
}

// ── Cloud sync / migration (T18 will fulfil) ────────────────────────────────

export interface MigrationProgressEvent {
  attempted: number;
  flushed: number;
  /** Total rows the migration plans to upload. Stable for the lifetime
   *  of a single migration run. */
  total: number;
  /** Set when the migration finished (success OR exhausted retries). */
  done?: boolean;
  /** Set on a fatal error; the UI surfaces this in a toast. */
  error?: string;
}

export interface CloudSyncFacade {
  /** Count rows that would be enqueued by a Local→Cloud migration. The
   *  Storage section displays this in the Local→Cloud confirmation
   *  dialog. */
  countLocalAttestations(): Promise<number>;
  /** Count rows currently held server-side for the active Google
   *  session. Used by the Cloud→Local confirmation dialog so the row
   *  count reflects the *cloud* inventory, not the local copy. T18 will
   *  replace the stub (which returns 0) with the real query. */
  countCloudAttestations(): Promise<number>;
  /** Enqueue every local row into `pending_uploads`. Returns
   *  immediately; progress is observed via `subscribeProgress`. */
  enqueueAll(): Promise<void>;
  /** Subscribe to live progress updates (driven by the SW alarm). */
  subscribeProgress(cb: (e: MigrationProgressEvent) => void): () => void;
  /** Cloud → Local: download every attestation as a COSE bundle + .md
   *  archive. Returns the assembled .zip bytes. */
  exportAll(): Promise<Uint8Array>;
}

// ── Settings (thin wrapper around `src/settings.ts`) ────────────────────────

export interface SettingsFacade {
  load(): Promise<SettingsV1>;
  update(patch: Partial<Omit<SettingsV1, "version">>): Promise<SettingsV1>;
}

// ── Top-level facade ────────────────────────────────────────────────────────

export interface OptionsRuntime {
  settings: SettingsFacade;
  auth: AuthFacade;
  identity: IdentityFacade;
  keyEscrow: KeyEscrowFacade;
  cloudSync: CloudSyncFacade;
  /** Build / version metadata — populated from `chrome.runtime.getManifest`
   *  in the production runtime; tests set whatever they like. */
  about: {
    version: string;
    buildHash?: string;
  };
}

let current: OptionsRuntime | null = null;

/** Production entrypoint sets this once at boot; tests inject stubs via
 *  {@link setOptionsRuntime} before mounting components. */
export function setOptionsRuntime(runtime: OptionsRuntime | null): void {
  current = runtime;
}

/** Accessor consumed by every section. Returns the registered runtime
 *  or throws when nothing has been wired — that branch is a programmer
 *  error (production code calls `setOptionsRuntime(createDefault…())`
 *  in `main.tsx`; tests call it in `beforeEach`). */
export function getOptionsRuntime(): OptionsRuntime {
  if (!current) {
    throw new Error(
      "OptionsRuntime not initialised — call setOptionsRuntime() before mounting <App />",
    );
  }
  return current;
}
