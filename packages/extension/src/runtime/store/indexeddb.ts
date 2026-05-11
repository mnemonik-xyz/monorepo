// IndexedDB-backed `AttestationStore` for the extension. Public surface
// mirrors the Rust trait `core/src/storage/traits.rs::AttestationStore`
// + `LineageStore` + the cloud-sync queue described in the tech-spec.
//
// Browser-only: uses the `idb` Promise wrapper around the platform IDB
// API. Tests substitute `fake-indexeddb/auto` for an in-memory backend.
//
// Authorization model (matches the Rust trait): `search` MANDATES an
// `owner_pubkey` filter — there is no anonymous / cross-tenant path.
// `findByTx` and `count` operate on indexes and leave authorization to
// the caller (popup, recall overlay).
//
// Cosine search runs scan-all in JS over the f32 embedding column. For
// the MVP target of ≤10k rows this is fast enough on a Chromebook
// (the test suite asserts ranking parity, not throughput); HNSW is
// backlog (D3 follow-up).

import { openDB, type IDBPDatabase } from "idb";
import {
  applyMigrations,
  DB_NAME,
  SCHEMA_VERSION,
  type MnemonikDb,
} from "./migrations.js";
import type {
  AttestationRow,
  LineageEdge,
  PendingUpload,
  SearchResult,
} from "./types.js";

/** Threshold above which we emit a `mnemonik:quota-warning` event. */
const QUOTA_WARN_THRESHOLD = 0.8;

export interface IndexedDbStoreOptions {
  /** Override the default `mnemonik` database name (tests use ephemeral
   *  names so per-spec isolation is automatic). */
  dbName?: string;
}

export class IndexedDbStore {
  private readonly dbName: string;
  private dbPromise: Promise<IDBPDatabase<MnemonikDb>> | null = null;
  private persistRequested = false;

  constructor(options: IndexedDbStoreOptions = {}) {
    this.dbName = options.dbName ?? DB_NAME;
  }

  /**
   * Lazily open / migrate the database. First write also requests
   * persistent storage from the platform (a no-op in jsdom / fake-IDB).
   */
  private async db(): Promise<IDBPDatabase<MnemonikDb>> {
    if (!this.dbPromise) {
      this.dbPromise = openDB<MnemonikDb>(this.dbName, SCHEMA_VERSION, {
        upgrade(db, oldVersion, _newVersion, tx) {
          applyMigrations(db, oldVersion, tx);
        },
      });
    }
    return this.dbPromise;
  }

  // ── AttestationStore ───────────────────────────────────────────────

  /** Persist or replace one attestation row. */
  async saveAttestation(row: AttestationRow): Promise<void> {
    if (!row.attestation_id) {
      throw new Error("saveAttestation: attestation_id is required");
    }
    if (!row.owner_pubkey) {
      throw new Error("saveAttestation: owner_pubkey is required");
    }
    const db = await this.db();
    await db.put("attestations", row);
    void this.maybeRequestPersistence();
  }

  /**
   * Direct primary-key lookup for `attestation_id`. Used by the
   * T18 cloud-sync drain to fetch the full row that a `pending_uploads`
   * entry references. Returns `undefined` when the row was deleted
   * out from under the queue — caller dequeues to break the loop.
   */
  async findById(attestationId: string): Promise<AttestationRow | undefined> {
    if (!attestationId) return undefined;
    const db = await this.db();
    return db.get("attestations", attestationId);
  }

  /**
   * Look up by either solana_tx or arweave_tx. Scans because neither
   * field has a unique index (cloud-mode rows can carry distinct values
   * yet share `local:<hash>` shapes for offline-first writes).
   */
  async findByTx(txId: string): Promise<AttestationRow | undefined> {
    if (!txId) return undefined;
    const db = await this.db();
    let cursor = await db.transaction("attestations").store.openCursor();
    while (cursor) {
      const v = cursor.value;
      if (v.solana_tx === txId || v.arweave_tx === txId) return v;
      cursor = await cursor.continue();
    }
    return undefined;
  }

  /** Number of attestations signed by `signer_pubkey`. */
  async count(signerPubkey: string): Promise<number> {
    const db = await this.db();
    return db.countFromIndex("attestations", "by_signer", signerPubkey);
  }

  /**
   * List every attestation row scoped to `owner_pubkey`. Returns raw
   * rows (NOT search-projected) so callers can read the COSE bytes +
   * lineage fields needed for migration / export. `limit` + `offset`
   * support paging when a future caller wants chunked iteration; today
   * the migration path takes all rows.
   */
  async listByOwner(
    ownerPubkey: string,
    limit = 10_000,
    offset = 0,
  ): Promise<AttestationRow[]> {
    if (!ownerPubkey) {
      throw new Error("listByOwner: ownerPubkey is mandatory");
    }
    const db = await this.db();
    const rows = await db.getAllFromIndex(
      "attestations",
      "by_owner",
      ownerPubkey,
    );
    return rows.slice(offset, offset + limit);
  }

  /**
   * Cosine-similarity top-K search scoped to `owner_pubkey`. Optional
   * `tags` filter is AND-applied (every supplied tag must be present).
   */
  async search(
    queryEmbedding: Float32Array,
    ownerPubkey: string,
    limit: number,
    opts: { tags?: string[] } = {},
  ): Promise<SearchResult[]> {
    if (!ownerPubkey) {
      throw new Error("search: ownerPubkey is mandatory");
    }
    if (limit <= 0) return [];
    const db = await this.db();
    const rows = await db.getAllFromIndex(
      "attestations",
      "by_owner",
      ownerPubkey,
    );
    const tagFilter = opts.tags ?? [];
    const filtered =
      tagFilter.length === 0
        ? rows
        : rows.filter((r) => tagFilter.every((t) => r.tags.includes(t)));
    const scored = filtered.map((row) => ({
      row,
      score: cosineSimilarity(queryEmbedding, row.embedding),
    }));
    scored.sort((a, b) => b.score - a.score);
    return scored.slice(0, limit).map(({ row, score }) => ({
      attestation_id: row.attestation_id,
      content: row.content,
      content_hash: row.content_hash,
      tags: [...row.tags],
      solana_tx: row.solana_tx,
      arweave_tx: row.arweave_tx,
      created_at: row.created_at,
      relevance_score: score,
    }));
  }

  // ── LineageStore ───────────────────────────────────────────────────

  async saveEdge(
    parentId: string,
    childId: string,
    depth: number,
  ): Promise<void> {
    if (!parentId || !childId) {
      throw new Error("saveEdge: parent and child ids required");
    }
    const db = await this.db();
    const edge: LineageEdge = {
      parent_id: parentId,
      child_id: childId,
      depth,
      created_at: new Date().toISOString(),
    };
    await db.put("lineage_edges", edge);
  }

  /** Returns `[parent_id, depth]` pairs ordered by insertion. */
  async getEdges(childId: string): Promise<Array<[string, number]>> {
    const db = await this.db();
    const rows = await db.getAllFromIndex("lineage_edges", "by_child", childId);
    return rows.map((r) => [r.parent_id, r.depth]);
  }

  /** Remove every edge that mentions `artifactId` (parent or child). */
  async clearEdges(artifactId: string): Promise<void> {
    const db = await this.db();
    const tx = db.transaction("lineage_edges", "readwrite");
    for (const indexName of ["by_child", "by_parent"] as const) {
      const idx = tx.store.index(indexName);
      let cursor = await idx.openCursor(IDBKeyRange.only(artifactId));
      while (cursor) {
        await cursor.delete();
        cursor = await cursor.continue();
      }
    }
    await tx.done;
  }

  // ── pending_uploads queue ──────────────────────────────────────────

  async enqueue(attestationId: string): Promise<void> {
    if (!attestationId) {
      throw new Error("enqueue: attestation_id required");
    }
    const db = await this.db();
    await db.put("pending_uploads", {
      attestation_id: attestationId,
      enqueued_at: new Date().toISOString(),
    });
  }

  async dequeue(attestationId: string): Promise<void> {
    const db = await this.db();
    await db.delete("pending_uploads", attestationId);
  }

  /** Pending rows in FIFO order (by `enqueued_at`). */
  async listPending(): Promise<PendingUpload[]> {
    const db = await this.db();
    return db.getAllFromIndex("pending_uploads", "by_enqueued_at");
  }

  // ── platform plumbing ──────────────────────────────────────────────

  /**
   * Best-effort `navigator.storage.persist()` request and quota check.
   * Fires `mnemonik:quota-warning` (CustomEvent) on `globalThis` when
   * `usage / quota > 0.8`. Idempotent: only runs once per instance.
   */
  private async maybeRequestPersistence(): Promise<void> {
    if (this.persistRequested) return;
    this.persistRequested = true;
    const nav = (globalThis as { navigator?: Navigator }).navigator;
    const storage = nav?.storage;
    if (!storage) return;
    try {
      if (typeof storage.persist === "function") {
        await storage.persist();
      }
      if (typeof storage.estimate === "function") {
        const est = await storage.estimate();
        if (
          typeof est.usage === "number" &&
          typeof est.quota === "number" &&
          est.quota > 0 &&
          est.usage / est.quota > QUOTA_WARN_THRESHOLD
        ) {
          dispatchQuotaWarning(est.usage, est.quota);
        }
      }
    } catch {
      // Permission denials / private mode stubs — non-fatal.
    }
  }
}

/**
 * Cosine similarity over two embeddings of (assumed) equal length.
 * Returns 0 if either vector is zero-length OR has zero norm so the
 * scoring path doesn't NaN.
 */
export function cosineSimilarity(a: Float32Array, b: Float32Array): number {
  const n = Math.min(a.length, b.length);
  if (n === 0) return 0;
  let dot = 0;
  let na = 0;
  let nb = 0;
  for (let i = 0; i < n; i++) {
    const av = a[i] ?? 0;
    const bv = b[i] ?? 0;
    dot += av * bv;
    na += av * av;
    nb += bv * bv;
  }
  const denom = Math.sqrt(na) * Math.sqrt(nb);
  return denom === 0 ? 0 : dot / denom;
}

function dispatchQuotaWarning(usage: number, quota: number): void {
  const target = globalThis as {
    dispatchEvent?: (e: Event) => boolean;
    CustomEvent?: typeof CustomEvent;
  };
  if (typeof target.dispatchEvent !== "function") return;
  const Ctor = target.CustomEvent;
  if (typeof Ctor !== "function") return;
  target.dispatchEvent(
    new Ctor("mnemonik:quota-warning", {
      detail: { usage, quota, ratio: usage / quota },
    }),
  );
}
