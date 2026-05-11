// Heavy-path runtime. Pulled in via dynamic import from `runtime.ts`
// so the popup's first-paint bundle does not include the WASM glue,
// the IndexedDB store, or the embedder worker. Each tab pays the cost
// for the action it triggers (sign / recall / verify) and nothing more.

import { IndexedDbStore } from "../runtime/store/indexeddb.js";
import type { AttestationRow, SearchResult } from "../runtime/store/types.js";
import type {
  SignMemoryArgs,
  SignMemoryResult,
  VerifyArgs,
  VerifyOutcome,
} from "./runtime.js";

/**
 * Popup-realm `IndexedDbStore` singleton. MV3 popups live in their own
 * document and reload on each open, so a module-level singleton here is
 * effectively a per-open cache. The service worker (background) does
 * NOT share this realm — if a background recall path is ever added,
 * it must build its own `IndexedDbStore` instance via the `runtime/
 * store/indexeddb.ts` constructor rather than importing this module.
 */
let store: IndexedDbStore | null = null;

function getStore(): IndexedDbStore {
  if (!store) {
    store = new IndexedDbStore();
  }
  return store;
}

/**
 * Subset of the popup runtime that lives behind the dynamic-import
 * boundary. Only the three heavyweight methods are exposed here; the
 * lightweight readers (identity / storage tier / active tab) are
 * handled directly in `runtime.ts` to avoid pulling WASM into the
 * popup's first-paint bundle.
 */
export interface RuntimePipeline {
  signMemory(args: SignMemoryArgs): Promise<SignMemoryResult>;
  recall(query: string, limit: number): Promise<SearchResult[]>;
  verify(args: VerifyArgs): Promise<VerifyOutcome>;
}

let pipeline: RuntimePipeline | null = null;

export function getRuntimePipeline(): RuntimePipeline {
  if (!pipeline) {
    pipeline = createPipeline();
  }
  return pipeline;
}

function createPipeline(): RuntimePipeline {
  return {
    async signMemory(args: SignMemoryArgs): Promise<SignMemoryResult> {
      const { content, tags, source } = args;
      if (!content || content.trim() === "") {
        throw new Error("signMemory: content must not be empty");
      }
      const identity = await loadIdentity();
      if (!identity) {
        throw new Error("signMemory: identity not configured");
      }

      // Lazy: WASM + embedder. The embedder cold-start can take ~3s the
      // first time it downloads the model; the popup shows a spinner.
      const [{ embedText }, cose] = await Promise.all([
        import("./embedder-bridge.js"),
        import("../runtime/sign/cose.js"),
      ]);
      const embedding = await embedText(content);

      // Mirror the server's MEMORY_V1 artifact shape (canonical CBOR
      // order is field-defined; we hand the object as-is to the WASM
      // canonicaliser which knows the field ordering).
      const compressed = await cose.compressEmbedding(embedding, 4);
      const created_at = new Date().toISOString();
      // content_hash is blake3 of the UTF-8 content bytes. The WASM
      // `sign_attestation_bundle` derives `artifact_id` from this and
      // builds the MEMORY_V1-shaped artifact internally before
      // canonicalising + signing in one call. Bypasses the broken
      // `to_canonical_cbor_bytes` path (serde_json::Value can't hold
      // Uint8Array, so embedding_compressed fails to deserialise).
      const contentBytes = new TextEncoder().encode(content);
      const content_hash = await cose.blake3HashHex(contentBytes);
      const cose_bytes = await cose.signAttestationBundle(
        content,
        compressed,
        content_hash,
        identity.pubkey_base58,
        { secret: identity.secret, pubkey_base58: identity.pubkey_base58 },
      );
      const truncated = content_hash.slice(0, 16);
      const tx = `local:${truncated}`;
      const attestation_id = `att_${truncated}`;
      const row: AttestationRow = {
        attestation_id,
        content,
        content_hash,
        tags,
        embedding,
        cose_bytes,
        created_at,
        signer_pubkey: identity.pubkey_base58,
        owner_pubkey: identity.pubkey_base58,
        solana_tx: tx,
        arweave_tx: tx,
        ...(source ? { source_meta: source } : {}),
      };
      await getStore().saveAttestation(row);
      return {
        attestation_id,
        content_hash,
        signer_pubkey: identity.pubkey_base58,
        solana_tx: tx,
        arweave_tx: tx,
        created_at,
      };
    },

    async recall(query: string, limit: number): Promise<SearchResult[]> {
      const trimmed = query.trim();
      if (trimmed === "" || limit <= 0) return [];
      const identity = await loadIdentity();
      if (!identity) return [];
      const { embedText } = await import("./embedder-bridge.js");
      const vector = await embedText(trimmed);
      return getStore().search(vector, identity.pubkey_base58, limit);
    },

    async verify(args: VerifyArgs): Promise<VerifyOutcome> {
      if (args.attestationId) {
        const id = args.attestationId.trim();
        if (!id) {
          return { status: "not_found" };
        }
        // The store keys attestations by `attestation_id`; today we only
        // expose `findByTx`, so for `att_<hash>` we synthesise the
        // matching `local:<hash>` tx-id and look up via that. T18 will
        // add a direct `findById` helper for cloud rows.
        const txCandidate = `local:${id.replace(/^att_/, "")}`;
        const row = await getStore().findByTx(txCandidate);
        if (!row) {
          return { status: "not_found", attestation_id: id };
        }
        return verifyRow(row);
      }
      if (args.fileBytes && args.fileBytes.length > 0) {
        // Phase 1 popup: file-drop verification is not yet implemented.
        // The WASM `verify_artifact` export is a pending T05 follow-up;
        // until then we surface the not-yet-supported state explicitly
        // so the UI can render a clear placeholder ("paste an
        // attestation id — file-drop coming soon") instead of the
        // misleading generic "not found" outcome the round-1 review
        // flagged.
        return { status: "not_found", reason: "file_drop_unsupported" };
      }
      return { status: "not_found" };
    },
  };
}

function verifyRow(row: AttestationRow): VerifyOutcome {
  // Round-2 fix (MAJ-2): the popup does NOT yet perform cryptographic
  // signature verification — the WASM `verify_artifact` export is a
  // pending T05 follow-up. Until it lands we run a presence check
  // (row exists + COSE envelope is non-empty) and mark the result
  // `presence_only: true`. The UI renders a "STORED LOCALLY —
  // cryptographic verification coming soon" caveat instead of a plain
  // green "VERIFIED" banner so the user is not misled. Missing COSE
  // bytes still trip the tampered path.
  if (!row.cose_bytes || row.cose_bytes.length === 0) {
    return { status: "tampered", reason: "missing COSE envelope" };
  }
  const result: VerifyOutcome = {
    status: "verified",
    presence_only: true,
    signer_pubkey: row.signer_pubkey,
    created_at: row.created_at,
    content_hash: row.content_hash,
    ...(row.source_meta ? { source: row.source_meta } : {}),
    ...(row.solana_tx ? { solana_tx: row.solana_tx } : {}),
    ...(row.arweave_tx ? { arweave_tx: row.arweave_tx } : {}),
  };
  return result;
}

interface FullIdentity {
  pubkey_base58: string;
  secret: number[];
}

async function loadIdentity(): Promise<FullIdentity | null> {
  // Goes through the same defensive read pattern as `runtime.ts`'s
  // `readChromeStorage` (try/catch around `chrome.storage.local.get` so
  // the popup never crashes on storage permission errors). Also guards
  // against a truncated `identity_secret` write — Solana keypairs are
  // 64 bytes (32-byte seed || 32-byte pubkey); anything else is bogus
  // and would produce malformed signatures if we let it through.
  let stored: {
    identity?: { pubkey_base58?: string } | null;
    identity_secret?: number[] | null;
  } = {};
  try {
    stored = (await chrome.storage.local.get([
      "identity",
      "identity_secret",
    ])) as typeof stored;
  } catch {
    return null;
  }
  const pub = stored.identity?.pubkey_base58;
  const sec = stored.identity_secret;
  if (!pub || !Array.isArray(sec) || sec.length !== 64) return null;
  return { pubkey_base58: pub, secret: sec };
}
