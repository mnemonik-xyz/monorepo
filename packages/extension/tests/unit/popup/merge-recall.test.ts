// Unit tests for `mergeRecallHits` — the Cloud-tier Recall merge.
// Lives next to `merge-recall.ts` (not embedded in the Recall
// component test) so the pure logic is auditable without React +
// jsdom + IndexedDbStore. Covers the cases enumerated by code-review
// round-1 finding T18-C-05: local-only, cloud-only, both / dedup,
// cloud-tx-wins-over-local-prefix, re-rank by score descending.

import { describe, it, expect } from "vitest";

import { mergeRecallHits } from "../../../src/popup/util/merge-recall.js";
import type { SearchResult } from "../../../src/runtime/store/types.js";
import type { CloudRecallHit } from "../../../src/popup/runtime.js";

// Factories — keep test cases concise; mirror Recall.test.tsx fixtures.
function localHit(over: Partial<SearchResult>): SearchResult {
  return {
    attestation_id: over.attestation_id ?? "att",
    content: over.content ?? "local content",
    content_hash: over.content_hash ?? "0".repeat(64),
    tags: over.tags ?? [],
    solana_tx: over.solana_tx ?? "local:x",
    arweave_tx: over.arweave_tx ?? "local:x",
    created_at: over.created_at ?? "2026-05-10T00:00:00Z",
    relevance_score: over.relevance_score ?? 0.5,
  };
}

function cloudHit(over: Partial<CloudRecallHit>): CloudRecallHit {
  return {
    attestation_id: over.attestation_id ?? "att",
    content: over.content ?? "cloud content",
    similarity: over.similarity ?? 0.5,
    ...(over.tags !== undefined ? { tags: over.tags } : {}),
    ...(over.signed_at !== undefined ? { signed_at: over.signed_at } : {}),
    ...(over.solana_tx !== undefined ? { solana_tx: over.solana_tx } : {}),
    ...(over.arweave_tx !== undefined ? { arweave_tx: over.arweave_tx } : {}),
  };
}

describe("mergeRecallHits · local-only", () => {
  it("returns the local slice unchanged when cloud is null", () => {
    const local = [
      localHit({ attestation_id: "a", relevance_score: 0.9 }),
      localHit({ attestation_id: "b", relevance_score: 0.7 }),
    ];
    expect(mergeRecallHits(local, null, 5)).toEqual(local);
  });

  it("returns the local slice unchanged when cloud is empty", () => {
    const local = [localHit({ attestation_id: "a", relevance_score: 0.9 })];
    expect(mergeRecallHits(local, [], 5)).toEqual(local);
  });

  it("truncates to the limit", () => {
    const local = [
      localHit({ attestation_id: "a", relevance_score: 0.9 }),
      localHit({ attestation_id: "b", relevance_score: 0.8 }),
      localHit({ attestation_id: "c", relevance_score: 0.7 }),
    ];
    const out = mergeRecallHits(local, null, 2);
    expect(out.map((r) => r.attestation_id)).toEqual(["a", "b"]);
  });
});

describe("mergeRecallHits · cloud-only", () => {
  it("projects cloud-only entries into SearchResult shape", () => {
    const cloud = [
      cloudHit({
        attestation_id: "cloud-only",
        content: "only on server",
        similarity: 0.7,
        tags: ["t1"],
        signed_at: "2026-05-11T00:00:00Z",
        solana_tx: "Sol",
        arweave_tx: "Ar",
      }),
    ];
    expect(mergeRecallHits([], cloud, 5)).toEqual([
      {
        attestation_id: "cloud-only",
        content: "only on server",
        content_hash: "",
        tags: ["t1"],
        solana_tx: "Sol",
        arweave_tx: "Ar",
        created_at: "2026-05-11T00:00:00Z",
        relevance_score: 0.7,
      },
    ]);
  });

  it("drops cloud hits with an empty attestation_id", () => {
    const out = mergeRecallHits([], [cloudHit({ attestation_id: "" })], 5);
    expect(out).toEqual([]);
  });
});

describe("mergeRecallHits · both / dedup", () => {
  it("dedupes by attestation_id, keeping the higher score (cloud wins)", () => {
    const out = mergeRecallHits(
      [localHit({ attestation_id: "a", relevance_score: 0.5 })],
      [cloudHit({ attestation_id: "a", similarity: 0.9 })],
      5,
    );
    expect(out).toHaveLength(1);
    expect(out[0]?.relevance_score).toBe(0.9);
  });

  it("dedupes by attestation_id, keeping the higher score (local wins)", () => {
    const out = mergeRecallHits(
      [localHit({ attestation_id: "a", relevance_score: 0.9 })],
      [cloudHit({ attestation_id: "a", similarity: 0.4 })],
      5,
    );
    expect(out).toHaveLength(1);
    expect(out[0]?.relevance_score).toBe(0.9);
  });
});

describe("mergeRecallHits · cloud-tx-wins-over-local-prefix", () => {
  it("folds cloud tx ids over synthetic local: ids", () => {
    const out = mergeRecallHits(
      [
        localHit({
          attestation_id: "a",
          relevance_score: 0.5,
          solana_tx: "local:a",
          arweave_tx: "local:a",
        }),
      ],
      [
        cloudHit({
          attestation_id: "a",
          similarity: 0.6,
          solana_tx: "RealSol",
          arweave_tx: "RealAr",
        }),
      ],
      5,
    );
    expect(out[0]?.solana_tx).toBe("RealSol");
    expect(out[0]?.arweave_tx).toBe("RealAr");
  });

  it("keeps real local tx ids when cloud has different real ones", () => {
    const out = mergeRecallHits(
      [
        localHit({
          attestation_id: "a",
          solana_tx: "OldRealSol",
          arweave_tx: "OldRealAr",
        }),
      ],
      [
        cloudHit({
          attestation_id: "a",
          solana_tx: "NewRealSol",
          arweave_tx: "NewRealAr",
        }),
      ],
      5,
    );
    expect(out[0]?.solana_tx).toBe("OldRealSol");
    expect(out[0]?.arweave_tx).toBe("OldRealAr");
  });

  it("does NOT overwrite local prefixes when cloud omits tx fields", () => {
    const out = mergeRecallHits(
      [
        localHit({
          attestation_id: "a",
          solana_tx: "local:a",
          arweave_tx: "local:a",
        }),
      ],
      [cloudHit({ attestation_id: "a" })],
      5,
    );
    expect(out[0]?.solana_tx).toBe("local:a");
    expect(out[0]?.arweave_tx).toBe("local:a");
  });
});

describe("mergeRecallHits · re-rank by score descending", () => {
  it("interleaves cloud-only and local entries by score", () => {
    const local = [
      localHit({ attestation_id: "low", relevance_score: 0.2 }),
      localHit({ attestation_id: "high", relevance_score: 0.9 }),
    ];
    const cloud = [cloudHit({ attestation_id: "mid", similarity: 0.5 })];
    const out = mergeRecallHits(local, cloud, 5);
    expect(out.map((r) => r.attestation_id)).toEqual(["high", "mid", "low"]);
  });
});
