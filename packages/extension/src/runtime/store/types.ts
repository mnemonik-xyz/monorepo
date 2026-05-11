// TypeScript shapes mirroring `core/src/storage/traits.rs`.
// Field names match the Rust SQL schema verbatim so a row round-trips
// across SQLite (server) and IndexedDB (extension) without renaming.

export interface AttestationRow {
  /** Stable attestation identifier — opaque from the storage perspective. */
  attestation_id: string;
  /** Plaintext memory content. UTF-8. */
  content: string;
  /**
   * Lowercase hex blake3 digest, 64 chars.
   *
   * NOTE (PR134-C-01 / TODO(T18-content-hash)): the extension currently
   * computes this as `blake3(utf8 content bytes)` while the server
   * (`mcp/src/tools.rs::sign_memory`) computes it as
   * `blake3(canonical_cbor(artifact))`. The two values are NOT
   * equal — the extension's live-debug fix bypassed the broken
   * `to_canonical_cbor_bytes` round-trip by hashing the raw UTF-8
   * payload instead. The cloud-sync drain still uploads the COSE
   * envelope (which contains the server-derived hash) so the
   * server-side row is correct; the LOCAL `content_hash` field is
   * only used for `attestation_id` derivation and Verify-tab UI.
   *
   * Cloud-sync round-trip MUST be re-verified after T18-cloud-client
   * server-side change exposes a parallel `attest_full` WASM entry
   * point that returns both the canonical-CBOR hash and the COSE
   * bytes. Until then, do not assume this hash matches the
   * server-side `content_hash` column. */
  content_hash: string;
  /** Free-form tags, persisted with multi-entry index for AND filters. */
  tags: string[];
  /**
   * Uncompressed f32 embedding. The server stores the *compressed* bytes
   * on Arweave and the f32 vector in SQLite (per `core/src/storage/sqlite.rs`);
   * we mirror the latter so cosine search runs without a decompress step.
   */
  embedding: Float32Array;
  /** COSE_Sign1 envelope bytes. Verified offline via `verify_artifact`. */
  cose_bytes: Uint8Array;
  /** RFC-3339 timestamp the server assigned on `sign-callback`. */
  created_at: string;
  /** Base58 Ed25519 signer pubkey (the COSE `kid`). */
  signer_pubkey: string;
  /** Base58 owner / tenant pubkey — `search` filters on this. */
  owner_pubkey: string;
  /** Solana SPL Memo tx id, or `local:<truncated_hash>` in local-only mode. */
  solana_tx: string;
  /** Arweave bundle tx id, or `local:<truncated_hash>` in local-only mode. */
  arweave_tx: string;
  /** Optional capture-context metadata for the popup recall view. */
  source_meta?: SourceMeta;
  /**
   * Set by the T18 cloud-sync drain when the server rejects this row
   * with a non-recoverable 4xx (e.g. 410 expired correlation_id, 400
   * malformed payload). The row stays in `attestations` so the user
   * sees a "sync failed — recreate this memory" hint in the popup;
   * `pending_uploads` is dequeued so the alarm-drain stops retrying.
   *
   * Optional + nullable; absence means "never attempted or transiently
   * failing". 401 does NOT set this — re-auth is recoverable.
   */
  sync_failed_permanent?: boolean;
}

/** Attached when the row was captured from a supported AI-chat platform. */
export interface SourceMeta {
  /** `chatgpt` | `claude` | `gemini` | other adapter platform name. */
  platform: string;
  /** Best-effort model hint (e.g. `gpt-4o`, `claude-3.5-sonnet`). */
  model?: string;
  /** Stable per-chat id when the platform exposes one. */
  chat_id?: string;
  /** Source URL the capture came from. */
  url?: string;
}

/** Output shape of `IndexedDbStore.search`. */
export interface SearchResult {
  attestation_id: string;
  content: string;
  content_hash: string;
  tags: string[];
  solana_tx: string;
  arweave_tx: string;
  created_at: string;
  /** Cosine similarity in [-1, 1]; higher = more relevant. */
  relevance_score: number;
}

/** Lineage edge — `child_id` was derived from / superseded `parent_id`. */
export interface LineageEdge {
  /** Auto-incrementing surrogate primary key. */
  id?: number;
  parent_id: string;
  child_id: string;
  depth: number;
  created_at: string;
}

/** Row in the cloud-sync queue. Drained by the alarm-triggered worker. */
export interface PendingUpload {
  /** PK — same id as the attestation row this upload would replicate. */
  attestation_id: string;
  enqueued_at: string;
}
