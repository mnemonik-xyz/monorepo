// TypeScript shapes mirroring `core/src/storage/traits.rs`.
// Field names match the Rust SQL schema verbatim so a row round-trips
// across SQLite (server) and IndexedDB (extension) without renaming.

export interface AttestationRow {
  /** Stable attestation identifier — opaque from the storage perspective. */
  attestation_id: string;
  /** Plaintext memory content. UTF-8. */
  content: string;
  /** Lowercase hex blake3 of the canonical CBOR. 64 chars. */
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
