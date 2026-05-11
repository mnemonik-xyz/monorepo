// IndexedDbStore tests — mirrors `core/src/storage/sqlite.rs` test
// structure so a Rust/TS contract drift fails one of these.
//
// Backend: `fake-indexeddb/auto` patches the platform IDB API into the
// vitest realm. Each `describe` opens a uniquely-named database so no
// cross-test cleanup is needed.

import "fake-indexeddb/auto";
import { describe, it, expect, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  cosineSimilarity,
  IndexedDbStore,
} from "../../../src/runtime/store/indexeddb.js";
import { applyMigrations } from "../../../src/runtime/store/migrations.js";
import type { AttestationRow } from "../../../src/runtime/store/types.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PATH = path.resolve(
  __dirname,
  "../../fixtures/store/cosine-fixture.json",
);

let dbCounter = 0;
function freshDbName(label: string): string {
  dbCounter += 1;
  return `mnemonik-test-${label}-${dbCounter}`;
}

/** Build a minimal valid AttestationRow for tests. */
function makeRow(over: Partial<AttestationRow>): AttestationRow {
  const id = over.attestation_id ?? "att:default";
  const dim = over.embedding?.length ?? 8;
  return {
    attestation_id: id,
    content: over.content ?? `content of ${id}`,
    content_hash: over.content_hash ?? "0".repeat(64),
    tags: over.tags ?? [],
    embedding: over.embedding ?? new Float32Array(dim),
    cose_bytes: over.cose_bytes ?? new Uint8Array([0x84, 0x01, 0x02]),
    created_at: over.created_at ?? "2026-05-10T00:00:00Z",
    signer_pubkey: over.signer_pubkey ?? "SignerA",
    owner_pubkey: over.owner_pubkey ?? over.signer_pubkey ?? "OwnerA",
    solana_tx: over.solana_tx ?? `local:${id}`,
    arweave_tx: over.arweave_tx ?? `local:${id}`,
    ...(over.source_meta ? { source_meta: over.source_meta } : {}),
  };
}

describe("cosineSimilarity (pure)", () => {
  it("returns 1 for identical unit vectors", () => {
    const v = new Float32Array([1, 0, 0]);
    expect(cosineSimilarity(v, v)).toBeCloseTo(1, 9);
  });

  it("returns 0 for orthogonal vectors", () => {
    expect(
      cosineSimilarity(
        new Float32Array([1, 0, 0]),
        new Float32Array([0, 1, 0]),
      ),
    ).toBe(0);
  });

  it("handles zero-norm input without NaN", () => {
    expect(
      cosineSimilarity(
        new Float32Array([0, 0, 0]),
        new Float32Array([1, 0, 0]),
      ),
    ).toBe(0);
  });
});

describe("IndexedDbStore · CRUD + indexes", () => {
  it("save → findByTx round-trips by solana_tx and arweave_tx", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("crud") });
    const row = makeRow({
      attestation_id: "att:1",
      solana_tx: "5j7sDe...",
      arweave_tx: "abc123",
    });
    await store.saveAttestation(row);
    const bySol = await store.findByTx("5j7sDe...");
    expect(bySol?.attestation_id).toBe("att:1");
    const byArw = await store.findByTx("abc123");
    expect(byArw?.attestation_id).toBe("att:1");
    expect(await store.findByTx("nope")).toBeUndefined();
  });

  it("count returns rows by signer_pubkey index", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("count") });
    await store.saveAttestation(
      makeRow({ attestation_id: "a", signer_pubkey: "S1", owner_pubkey: "O1" }),
    );
    await store.saveAttestation(
      makeRow({ attestation_id: "b", signer_pubkey: "S1", owner_pubkey: "O1" }),
    );
    await store.saveAttestation(
      makeRow({ attestation_id: "c", signer_pubkey: "S2", owner_pubkey: "O1" }),
    );
    expect(await store.count("S1")).toBe(2);
    expect(await store.count("S2")).toBe(1);
    expect(await store.count("missing")).toBe(0);
  });
});

describe("IndexedDbStore · search", () => {
  it("cosine_search_top_k_matches_python_fixture: known top-3 ordering", async () => {
    const fixture = JSON.parse(readFileSync(FIXTURE_PATH, "utf8")) as {
      vectors: Array<{ id: string; embedding: number[] }>;
      query: number[];
      expected_top_3: string[];
    };
    const store = new IndexedDbStore({ dbName: freshDbName("cosine") });
    for (const v of fixture.vectors) {
      await store.saveAttestation(
        makeRow({
          attestation_id: v.id,
          embedding: new Float32Array(v.embedding),
          owner_pubkey: "Tenant",
          signer_pubkey: "Tenant",
        }),
      );
    }
    const hits = await store.search(
      new Float32Array(fixture.query),
      "Tenant",
      3,
    );
    expect(hits.map((h) => h.attestation_id)).toEqual(fixture.expected_top_3);
    // Scores monotonically descending.
    expect(hits[0]!.relevance_score).toBeGreaterThan(hits[1]!.relevance_score);
    expect(hits[1]!.relevance_score).toBeGreaterThan(hits[2]!.relevance_score);
  });

  it("owner_scope_isolates_tenants: search never returns other owner's rows", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("owners") });
    for (let i = 0; i < 4; i++) {
      await store.saveAttestation(
        makeRow({
          attestation_id: `a${i}`,
          owner_pubkey: "OwnerA",
          embedding: new Float32Array([1, i / 10, 0, 0]),
        }),
      );
    }
    for (let i = 0; i < 3; i++) {
      await store.saveAttestation(
        makeRow({
          attestation_id: `b${i}`,
          owner_pubkey: "OwnerB",
          embedding: new Float32Array([1, i / 10, 0, 0]),
        }),
      );
    }
    const aHits = await store.search(
      new Float32Array([1, 0, 0, 0]),
      "OwnerA",
      10,
    );
    expect(aHits.length).toBe(4);
    expect(aHits.every((h) => h.attestation_id.startsWith("a"))).toBe(true);

    const bHits = await store.search(
      new Float32Array([1, 0, 0, 0]),
      "OwnerB",
      10,
    );
    expect(bHits.length).toBe(3);
    expect(bHits.every((h) => h.attestation_id.startsWith("b"))).toBe(true);

    expect(
      await store.search(new Float32Array([1, 0, 0, 0]), "Unknown", 10),
    ).toEqual([]);
  });

  it("tag filter narrows results (AND semantics)", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("tags") });
    await store.saveAttestation(
      makeRow({
        attestation_id: "x",
        tags: ["work", "ai"],
        embedding: new Float32Array([1, 0, 0, 0]),
      }),
    );
    await store.saveAttestation(
      makeRow({
        attestation_id: "y",
        tags: ["work"],
        embedding: new Float32Array([1, 0, 0, 0]),
      }),
    );
    await store.saveAttestation(
      makeRow({
        attestation_id: "z",
        tags: ["personal"],
        embedding: new Float32Array([1, 0, 0, 0]),
      }),
    );
    const hits = await store.search(
      new Float32Array([1, 0, 0, 0]),
      "OwnerA",
      10,
      { tags: ["work", "ai"] },
    );
    expect(hits.map((h) => h.attestation_id)).toEqual(["x"]);
  });

  it("limit ≤ 0 short-circuits to []", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("limit") });
    await store.saveAttestation(makeRow({ attestation_id: "a" }));
    expect(
      await store.search(new Float32Array([1, 0, 0, 0]), "OwnerA", 0),
    ).toEqual([]);
  });

  it("rejects empty owner_pubkey", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("noowner") });
    await expect(
      store.search(new Float32Array([1]), "", 5),
    ).rejects.toThrow(/ownerPubkey is mandatory/);
  });
});

describe("IndexedDbStore · lineage", () => {
  it("save → getEdges round-trips and clearEdges removes both sides", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("lineage") });
    await store.saveEdge("p1", "c1", 1);
    await store.saveEdge("p2", "c1", 2);
    await store.saveEdge("c1", "grandchild", 1);
    const edges = await store.getEdges("c1");
    expect(edges.length).toBe(2);
    expect(edges).toEqual(
      expect.arrayContaining([
        ["p1", 1],
        ["p2", 2],
      ]),
    );
    await store.clearEdges("c1");
    expect(await store.getEdges("c1")).toEqual([]);
    // The grandchild → c1 edge is also gone (c1 was the parent there).
    expect(await store.getEdges("grandchild")).toEqual([]);
  });
});

describe("IndexedDbStore · pending_uploads queue", () => {
  it("enqueue → listPending → dequeue", async () => {
    const store = new IndexedDbStore({ dbName: freshDbName("queue") });
    await store.enqueue("att:1");
    await store.enqueue("att:2");
    let pending = await store.listPending();
    expect(pending.map((p) => p.attestation_id).sort()).toEqual([
      "att:1",
      "att:2",
    ]);
    await store.dequeue("att:1");
    pending = await store.listPending();
    expect(pending.map((p) => p.attestation_id)).toEqual(["att:2"]);
  });
});

describe("IndexedDbStore · schema migration", () => {
  // The synthetic v0 → v1 path: applyMigrations is invoked with
  // oldVersion=0 inside `openDB`'s `upgrade` callback. Asserting the
  // post-state is the closest we can get without committing real
  // legacy fixtures.
  it("v0 → v1 creates the three object stores with required indexes", async () => {
    // Open with `idb` to invoke the same migration path the production
    // store uses, then inspect the resulting schema.
    const dbName = freshDbName("migration");
    const { openDB } = await import("idb");
    const db = await openDB(dbName, 1, {
      upgrade(db, oldVersion, _newVersion, tx) {
        expect(oldVersion).toBe(0);
        applyMigrations(db as never, oldVersion, tx as never);
      },
    });
    const stores = Array.from(db.objectStoreNames).sort();
    expect(stores).toEqual([
      "attestations",
      "lineage_edges",
      "pending_uploads",
    ]);
    const attTx = db.transaction("attestations");
    const indexes = Array.from(attTx.store.indexNames).sort();
    expect(indexes).toEqual([
      "by_created_at",
      "by_owner",
      "by_signer",
      "by_tags",
    ]);
    expect(attTx.store.index("by_tags").multiEntry).toBe(true);
    db.close();
  });
});
