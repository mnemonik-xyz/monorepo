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
   */
  auth: {
    signIn(): Promise<GoogleSignInResult>;
    lookupExisting(jwt: string): Promise<LookupResult>;
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
    async signRemote() {
      // T18 wires the cloud client. Local-tier and pre-T18 builds are
      // both fine to no-op here.
      return;
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
    },
  };
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
