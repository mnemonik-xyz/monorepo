// Popup-facing runtime facade. Glues the chat-adapter registry (T06),
// the IndexedDB store (T03), the WASM signing pipeline (T05), and the
// embedder worker (T04) into the three actions the popup needs:
// `signMemory`, `recall`, `verify`. Concrete implementations are wired
// here; tests replace the exported `setRuntime` function to inject
// mocks (the popup never imports the implementation directly).
//
// `signRemote` is a no-op stub on Local tier. T18 will replace it with
// the deferred-signing cloud-client; until then Cloud-tier writes fall
// back to Local + the pending_uploads queue.

import type { ChatTurn, ChatAdapter } from "../runtime/chat/types.js";
import type { SearchResult, SourceMeta } from "../runtime/store/types.js";
import type {
  GoogleSignInResult,
  LookupResult,
  Session,
} from "../auth/types.js";
import type { EscrowBlob } from "../auth/key-escrow.js";
import type {
  RecallRemoteHit,
  VerifyRemoteResult,
} from "../runtime/sync/cloud-client.js";
// Sync JWT-parsing helper — exposed on the runtime facade so popup
// components don't import `auth/session.ts` directly (M3 / AUD-C-08).
import { jwtExpiresAtMs as syncJwtExpiresAtMs } from "../auth/session.js";

/** Re-export of `RecallRemoteHit` so popup callers depend on the
 *  facade rather than reaching into `runtime/sync/cloud-client.ts`. */
export type CloudRecallHit = RecallRemoteHit;
/** Same re-export pattern for the verify-result discriminated union. */
export type CloudVerifyOutcome = VerifyRemoteResult;

/** Identity loaded out of `chrome.storage.local`. `null` when the user
 *  has not completed T16 onboarding yet (popup renders a "not signed
 *  in" badge in that case). */
export interface PopupIdentity {
  pubkey_base58: string;
  /** Optional human-readable label (T16 sets this from Google sign-in). */
  label?: string;
}

export type StorageTier = "local" | "cloud";

export interface SignMemoryArgs {
  content: string;
  tags: string[];
  source?: SourceMeta;
}

export interface SignMemoryResult {
  attestation_id: string;
  content_hash: string;
  signer_pubkey: string;
  solana_tx: string;
  arweave_tx: string;
  created_at: string;
}

export type VerifyOutcome =
  | {
      status: "verified";
      signer_pubkey: string;
      created_at: string;
      content_hash: string;
      source?: SourceMeta;
      solana_tx?: string;
      arweave_tx?: string;
      /**
       * `true` when the popup only confirmed that the local row exists
       * + has a non-empty COSE envelope (presence check), without
       * running the cryptographic signature verification. The WASM
       * `verify_artifact` export is pending (T05 follow-up); until it
       * lands, the UI renders a "STORED LOCALLY — cryptographic
       * verification coming soon" caveat instead of a plain green
       * "VERIFIED" banner. Cloud-path verification (T18) will return
       * this field `false` once full cryptographic verify is wired.
       */
      presence_only?: boolean;
    }
  | { status: "tampered"; reason: string }
  | {
      status: "not_found";
      attestation_id?: string;
      /**
       * Set when the popup received `fileBytes` instead of an
       * attestation id. File-drop verification is not yet implemented
       * (T05 follow-up). The UI renders a clear placeholder message
       * rather than the generic "not found" state to avoid misleading
       * the user.
       */
      reason?: "file_drop_unsupported";
    };

export interface VerifyArgs {
  /** Either an `attestation_id` string OR raw COSE_Sign1 bytes from a
   *  dropped file. The popup hands one or the other; the runtime decides
   *  which path to take. */
  attestationId?: string;
  fileBytes?: Uint8Array;
}

/** Abstract runtime contract the popup consumes. Real impl lives at the
 *  bottom of this file; tests inject a stub via `setRuntime`. */
export interface PopupRuntime {
  loadIdentity(): Promise<PopupIdentity | null>;
  loadStorageTier(): Promise<StorageTier>;
  /** Read the active tab URL and match against the adapter registry.
   *  Returns `null` when not on a supported chat page or the URL cannot
   *  be matched (the popup falls back to selection-only capture). */
  getActiveTabAdapter(): Promise<ChatAdapter | null>;
  /** Ask the active tab's content script for any current selection.
   *  Resolves to "" when nothing is selected or the active tab does not
   *  host a Mnemonik content script. Never throws — failures surface as
   *  empty strings so the popup textarea stays empty. */
  getActiveTabSelection(): Promise<string>;
  /** Ask the active tab's adapter to extract the current conversation.
   *  Returns `null` when no adapter matches the active tab. */
  getActiveTabConversation(): Promise<ChatTurn[] | null>;
  signMemory(args: SignMemoryArgs): Promise<SignMemoryResult>;
  /** T18 placeholder. On Local tier this is a no-op; on Cloud tier it
   *  would push the attestation to the hosted MCP server. */
  signRemote(args: SignMemoryArgs & SignMemoryResult): Promise<void>;
  recall(query: string, limit: number): Promise<SearchResult[]>;
  verify(args: VerifyArgs): Promise<VerifyOutcome>;

  /**
   * T16 auth surface. The popup never imports `src/auth/*` directly so
   * the runtime stays the single seam for tests + service-worker
   * integration. Heavy paths (`signInWithGoogle` opens
   * `chrome.identity.launchWebAuthFlow`, `lookupExisting` POSTs to the
   * server) sit behind dynamic imports in the default impl.
   *
   * Phase 1 hard-codes `DEFAULT_SERVER_ORIGIN` inside `signInWithGoogle`.
   * Multi-environment config (e.g. options.serverUrl read from
   * `chrome.storage.sync`) is a backlog item — when it lands, thread the
   * URL through this seam rather than re-exporting the auth helpers.
   */
  auth: {
    signIn(): Promise<GoogleSignInResult>;
    lookupExisting(jwt: string): Promise<LookupResult>;
    /**
     * Two-step bootstrap handshake (T15). Exchanges an `aud=mcp` JWT
     * for the short-lived `aud=extension` JWT that
     * `/api/key-escrow*` + `/api/sign-callback` require. Caches the
     * result in `chrome.storage.local.extension_session.v1` so a
     * popup-close / SW-restart does not re-burn a ticket. Callers
     * pass the result wherever they would have passed the
     * `aud=mcp` JWT against an escrow / sign-callback endpoint.
     */
    ensureExtensionJwt(mcpJwt: string): Promise<string>;
    /**
     * POST `/oauth/google/link` — bind the user's Ed25519 pubkey to
     * their Google `sub` via a possession-proof signature. Stays on
     * `aud=mcp` (the server's google-link handler runs the standard
     * `verify_jwt` validator). Audit M3 / AUD-C-08: SetPassphrase
     * routes through this seam instead of importing
     * `DEFAULT_SERVER_ORIGIN` / `AuthError` directly.
     */
    linkGoogle(
      mcpJwt: string,
      body: {
        pubkey_base58: string;
        possession_proof_base64: string;
        challenge: string;
      },
    ): Promise<void>;
  };

  /**
   * T16 session-store seam. Same dynamic-import pattern as `auth`.
   * Production code reads/writes `chrome.storage.local.session.v1` via
   * the helpers in `src/auth/session.ts`.
   */
  session: {
    get(): Promise<Session | null>;
    set(session: Session): Promise<void>;
    clear(): Promise<void>;
    /** Pure JWT-parsing helper — extracts the `exp` claim (in seconds)
     *  and returns it as milliseconds. Returns `null` for malformed
     *  tokens. Exposed through the facade (audit M3 / AUD-C-08) so
     *  popup components stay decoupled from `auth/session.ts`. */
    jwtExpiresAtMs(jwt: string): number | null;
  };

  /**
   * T18 cloud-sync seam. Popup tabs (Recall in Cloud mode, future
   * Verify-in-Cloud) route through this facade rather than importing
   * `runtime/sync/cloud-client.ts` directly. The concrete impl
   * dynamic-imports the client lazily so the popup's first-paint
   * bundle stays under the size-limit budget — only Recall in Cloud
   * mode pays the network-client cost.
   *
   * All three methods return `null` when there is no JWT-bearing
   * session yet; callers branch on that to render "sign in to see
   * cloud results" without throwing.
   */
  cloudSync: {
    signRemote(
      args: SignMemoryArgs & SignMemoryResult,
    ): Promise<{ solana_tx: string; arweave_tx: string } | null>;
    recallRemote(query: string, k: number): Promise<CloudRecallHit[] | null>;
    verifyRemote(attestationId: string): Promise<CloudVerifyOutcome | null>;
  };

  /**
   * T17 key-escrow seam. Popup onboarding components route through this
   * facade rather than importing `src/auth/key-escrow.ts` directly, so
   * tests have a single mock seam. The concrete impl dynamic-imports
   * `auth/key-escrow.ts` lazily to keep the popup's first-paint bundle
   * free of WebCrypto + hash-wasm.
   */
  keyEscrow: {
    /** Argon2id-derive a key and AES-GCM-256 encrypt `secret`. */
    wrap(
      secret: Uint8Array,
      passphrase: string,
      pubkeyBase58: string,
    ): Promise<EscrowBlob>;
    /** Argon2id-derive a key and AES-GCM-256 decrypt `blob`. Throws
     *  `WrongPassphraseError` on tag mismatch. Local-only — no network. */
    unwrap(blob: EscrowBlob, passphrase: string): Promise<Uint8Array>;
    /**
     * PUT the blob to `/api/key-escrow`.
     *
     * Callers pass their cached `aud=mcp` Google-session JWT; the
     * facade internally runs the T15 extension-bootstrap handshake and
     * substitutes the resulting `aud=extension` JWT before hitting the
     * server (audit B1 / AUD-S-01).
     */
    upload(mcpJwt: string, blob: EscrowBlob): Promise<void>;
    /** GET the blob from `/api/key-escrow`. See {@link upload} for the JWT contract. */
    fetch(mcpJwt: string): Promise<EscrowBlob>;
    /** DELETE the blob. See {@link upload} for the JWT contract. */
    delete(mcpJwt: string): Promise<void>;
    /** Fetch → unwrap with `oldPassphrase` → re-wrap → upload. See
     *  {@link upload} for the JWT contract. */
    rotate(
      mcpJwt: string,
      oldPassphrase: string,
      newPassphrase: string,
    ): Promise<void>;
    /** Probe via GET; treats 404 as `false`. See {@link upload} for the JWT contract. */
    hasBlob(mcpJwt: string): Promise<boolean>;
  };
}

let current: PopupRuntime | null = null;

/** Test-only entry point — production code never calls this. */
export function setRuntime(runtime: PopupRuntime | null): void {
  current = runtime;
}

/** Lazy accessor. Tests call `setRuntime` first; production callers
 *  fall through to {@link createDefaultRuntime}. */
export function getRuntime(): PopupRuntime {
  if (!current) {
    current = createDefaultRuntime();
  }
  return current;
}

// ── Default implementation ──────────────────────────────────────────────────

/**
 * Build the production runtime. Heavy paths (embedder worker, WASM
 * crypto) are imported lazily so the popup's initial JS stays under the
 * 50KB size-limit budget — Recall is the only tab that triggers the
 * embedder cold-start. The factory is exported so a future test that
 * wants the real pipeline (e.g. a fixture-only round-trip) can call it
 * directly without going through `getRuntime`.
 */
export function createDefaultRuntime(): PopupRuntime {
  return {
    async loadIdentity() {
      const stored = await readChromeStorage<{
        identity?: PopupIdentity | null;
      }>("local", ["identity"]);
      return stored.identity ?? null;
    },
    async loadStorageTier() {
      const stored = await readChromeStorage<{
        storage_tier?: StorageTier;
      }>("local", ["storage_tier"]);
      return stored.storage_tier ?? "local";
    },
    async getActiveTabAdapter() {
      const url = await activeTabUrl();
      if (!url) return null;
      const { selectAdapter } = await import("../runtime/chat/registry.js");
      // Side-effect import wires concrete adapters into the registry.
      await import("../runtime/chat/adapters/index.js");
      return selectAdapter(url);
    },
    async getActiveTabSelection() {
      const tabId = await activeTabId();
      if (typeof tabId !== "number") return "";
      try {
        const response = (await chrome.tabs.sendMessage(tabId, {
          type: "ui:get-selection",
        })) as { selectionText?: string } | undefined;
        return response?.selectionText ?? "";
      } catch {
        return "";
      }
    },
    async getActiveTabConversation() {
      const tabId = await activeTabId();
      if (typeof tabId !== "number") return null;
      try {
        const response = (await chrome.tabs.sendMessage(tabId, {
          type: "ui:extract-conversation",
        })) as { turns?: ChatTurn[] } | undefined;
        return response?.turns ?? null;
      } catch {
        return null;
      }
    },
    async signMemory(args) {
      const { getRuntimePipeline } = await import("./runtime-impl.js");
      return getRuntimePipeline().signMemory(args);
    },
    async signRemote(args) {
      // T18: Cloud-tier push. Task spec: attempt the live POST /mcp +
      // POST /api/sign-callback immediately, falling back to the
      // pending_uploads queue only on transient failure (network /
      // 5xx) or absent session. The alarm drain handles the queue.
      // Closes code-review round-1 finding T18-C-MAJOR-02.
      try {
        const { IndexedDbStore } =
          await import("../runtime/store/indexeddb.js");
        const store = new IndexedDbStore();
        // Always enqueue first so a crash between the live attempt
        // and its dequeue (browser tab close, popup dismiss) doesn't
        // strand the row outside the queue. The CloudClient's
        // happy path will `dequeue` immediately after the server
        // returns a 200; on failure the row stays for the alarm.
        await store.enqueue(args.attestation_id);
      } catch {
        // Best-effort enqueue. If IDB itself is unavailable, the
        // live push below will still try; failing both is fine.
      }
      try {
        const result = await this.cloudSync.signRemote(args);
        // null = no session bound — leave the row enqueued, the
        // user's next sign-in will pick it up on the alarm tick.
        // Non-null = success; cloudSync.signRemote already wrote
        // the tx ids back and dequeued. Nothing more to do.
        void result;
      } catch (err) {
        const cc = await import("../runtime/sync/cloud-client.js");
        if (err instanceof cc.ReauthRequiredError) {
          // Surface re-auth as a global event so the popup's App.tsx
          // can flip into onboarding mode. Leave the row queued so
          // the next alarm tick (after re-auth) retries it.
          const g = globalThis as {
            dispatchEvent?: (e: Event) => boolean;
            CustomEvent?: typeof CustomEvent;
          };
          if (
            typeof g.dispatchEvent === "function" &&
            typeof g.CustomEvent === "function"
          ) {
            try {
              g.dispatchEvent(new g.CustomEvent("mnemonik:re-auth-required"));
            } catch {
              // Read-only globals — non-fatal.
            }
          }
          return;
        }
        if (err instanceof cc.PermanentSyncError) {
          // Mark permanently failed locally so the popup can render
          // the "sync failed" hint. Dequeue so the alarm doesn't
          // churn. Mirror what `drainPendingUploads` does on the
          // background side.
          try {
            const { IndexedDbStore } =
              await import("../runtime/store/indexeddb.js");
            const store = new IndexedDbStore();
            const row = await store.findById(args.attestation_id);
            if (row) {
              await store.saveAttestation({
                ...row,
                sync_failed_permanent: true,
              });
            }
            await store.dequeue(args.attestation_id);
          } catch {
            // Best-effort cleanup.
          }
          // Re-throw so the caller (Capture.tsx) can surface a toast.
          throw err;
        }
        // TransientSyncError or unknown: leave the row queued. The
        // alarm drain will retry on the next tick. We don't re-throw
        // so the Capture happy-path UX isn't blocked by a transient
        // upstream hiccup — the user sees the local sign succeed.
      }
    },
    async recall(query, limit) {
      const { getRuntimePipeline } = await import("./runtime-impl.js");
      return getRuntimePipeline().recall(query, limit);
    },
    async verify(args) {
      const { getRuntimePipeline } = await import("./runtime-impl.js");
      return getRuntimePipeline().verify(args);
    },
    auth: {
      async signIn() {
        // Lazy: keeps `chrome.identity.launchWebAuthFlow` and the PKCE
        // helpers out of the popup's first-paint bundle.
        const { signInWithGoogle } = await import("../auth/google-oauth.js");
        return signInWithGoogle();
      },
      async lookupExisting(jwt: string) {
        const { lookupExisting } = await import("../auth/lookup.js");
        return lookupExisting(jwt);
      },
      async ensureExtensionJwt(mcpJwt: string) {
        const { ensureExtensionJwt } =
          await import("../auth/extension-bootstrap.js");
        return ensureExtensionJwt(mcpJwt);
      },
      async linkGoogle(mcpJwt, body) {
        const { linkGoogleAccount } = await import("../auth/link.js");
        return linkGoogleAccount(mcpJwt, body);
      },
    },
    session: {
      async get() {
        const { getSession } = await import("../auth/session.js");
        return getSession();
      },
      async set(session) {
        const { setSession } = await import("../auth/session.js");
        return setSession(session);
      },
      async clear() {
        const { clearSession } = await import("../auth/session.js");
        return clearSession();
      },
      jwtExpiresAtMs(jwt: string) {
        // Synchronous helper; not async-imported because callers
        // (Onboarding.tsx) call it inside the sign-in handler where a
        // dynamic import would force two await ticks. We accept the
        // tiny cost of bundling the JWT parser; it is <500 bytes
        // gzipped and parser-only (no chrome.* / WebCrypto deps).
        return syncJwtExpiresAtMs(jwt);
      },
    },
    cloudSync: {
      async signRemote(args) {
        // Synchronous push path. Tabs that want immediate cloud
        // confirmation (rather than waiting for the alarm-drain
        // tick) call this. Returns `null` when there is no session;
        // throws typed errors otherwise so the caller can branch.
        const client = await buildCloudClient();
        if (!client) return null;
        const { signer_pubkey } = args;
        // Re-load the COSE bytes off the local row — the popup
        // already persisted them in `signMemory`, so we don't have
        // to ship them across the messaging boundary again.
        const { IndexedDbStore } =
          await import("../runtime/store/indexeddb.js");
        const store = new IndexedDbStore();
        const row = await store.findById(args.attestation_id);
        if (!row) return null;
        const out = await client.signRemote(
          {
            content: row.content,
            tags: [...row.tags],
            ...(row.source_meta ? { source_meta: row.source_meta } : {}),
          },
          { cose_bytes: row.cose_bytes, signer_pubkey },
        );
        // Persist the tx ids back onto the row so subsequent Recall
        // calls render them. Mirrors what the alarm-drain does.
        await store.saveAttestation({
          ...row,
          solana_tx: out.solana_tx,
          arweave_tx: out.arweave_tx,
        });
        await store.dequeue(args.attestation_id);
        return { solana_tx: out.solana_tx, arweave_tx: out.arweave_tx };
      },
      async recallRemote(query, k) {
        const client = await buildCloudClient();
        if (!client) return null;
        return client.recallRemote(query, k);
      },
      async verifyRemote(attestationId) {
        const client = await buildCloudClient();
        if (!client) return null;
        return client.verifyRemote(attestationId);
      },
    },
    keyEscrow: {
      async wrap(secret, passphrase, pubkeyBase58) {
        const ke = await import("../auth/key-escrow.js");
        return ke.wrapSecret(secret, passphrase, pubkeyBase58);
      },
      async unwrap(blob, passphrase) {
        const ke = await import("../auth/key-escrow.js");
        return ke.unwrapSecret(blob, passphrase);
      },
      async upload(mcpJwt, blob) {
        // Audit B1 / AUD-S-01: server's escrow handlers require
        // aud=extension. Run the bootstrap handshake first; the
        // returned JWT is cached so subsequent calls are cheap.
        const { ensureExtensionJwt } =
          await import("../auth/extension-bootstrap.js");
        const extJwt = await ensureExtensionJwt(mcpJwt);
        const ke = await import("../auth/key-escrow.js");
        return ke.uploadEscrow(extJwt, blob);
      },
      async fetch(mcpJwt) {
        const { ensureExtensionJwt } =
          await import("../auth/extension-bootstrap.js");
        const extJwt = await ensureExtensionJwt(mcpJwt);
        const ke = await import("../auth/key-escrow.js");
        return ke.fetchEscrow(extJwt);
      },
      async delete(mcpJwt) {
        const { ensureExtensionJwt } =
          await import("../auth/extension-bootstrap.js");
        const extJwt = await ensureExtensionJwt(mcpJwt);
        const ke = await import("../auth/key-escrow.js");
        return ke.deleteEscrow(extJwt);
      },
      async rotate(mcpJwt, oldPassphrase, newPassphrase) {
        const { ensureExtensionJwt } =
          await import("../auth/extension-bootstrap.js");
        const extJwt = await ensureExtensionJwt(mcpJwt);
        const ke = await import("../auth/key-escrow.js");
        return ke.rotatePassphrase(extJwt, oldPassphrase, newPassphrase);
      },
      async hasBlob(mcpJwt) {
        const { ensureExtensionJwt } =
          await import("../auth/extension-bootstrap.js");
        const extJwt = await ensureExtensionJwt(mcpJwt);
        const ke = await import("../auth/key-escrow.js");
        try {
          await ke.fetchEscrow(extJwt);
          return true;
        } catch (err) {
          if (err instanceof ke.EscrowNotFoundError) return false;
          // Network / auth hiccups bubble to the caller — onboarding has
          // already exercised auth via `signIn`, so a 401 / 5xx here is
          // an actionable error rather than an "absent blob".
          throw err;
        }
      },
    },
  };
}

// ── cloud-sync helper ───────────────────────────────────────────────────────

/**
 * Build a `CloudClient` from the persisted session. Returns `null`
 * when no session is bound — callers branch on that to render
 * "sign in to see cloud results" without throwing. Dynamic-imports
 * to keep the popup's first-paint bundle free of `fetch` setup.
 */
async function buildCloudClient(): Promise<
  import("../runtime/sync/cloud-client.js").CloudClient | null
> {
  const { getSession } = await import("../auth/session.js");
  const session = await getSession();
  if (!session) return null;
  const { CloudClient } = await import("../runtime/sync/cloud-client.js");
  return new CloudClient({ jwt: session.jwt });
}

// ── chrome.* helpers (test-friendly) ────────────────────────────────────────

async function activeTabUrl(): Promise<string | null> {
  const tabs = await chrome.tabs.query({
    active: true,
    currentWindow: true,
  });
  return tabs[0]?.url ?? null;
}

async function activeTabId(): Promise<number | null> {
  const tabs = await chrome.tabs.query({
    active: true,
    currentWindow: true,
  });
  return tabs[0]?.id ?? null;
}

async function readChromeStorage<T>(
  area: "local" | "sync",
  keys: string[],
): Promise<Partial<T>> {
  try {
    const result = (await chrome.storage[area].get(
      keys as unknown as string,
    )) as unknown as Partial<T>;
    return result;
  } catch {
    return {};
  }
}
