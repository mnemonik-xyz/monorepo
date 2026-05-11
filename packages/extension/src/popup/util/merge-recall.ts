// Pure merge logic for Cloud-tier Recall: combine local IDB hits with
// remote MCP hits, dedupe by `attestation_id`, fold cloud-side tx ids
// over local synthetic placeholders, and re-rank by similarity. Lives
// in its own module so the logic is unit-testable without spinning up
// React + jsdom + IndexedDbStore mocks. Closes code-review round-1
// finding T18-C-05.

import type { SearchResult } from "../../runtime/store/types.js";
import type { CloudRecallHit } from "../runtime.js";

/**
 * Merge local IDB hits with cloud MCP hits.
 *
 * Behaviour:
 *   - `cloud === null || cloud.length === 0` → return `local.slice(0, limit)`.
 *   - Same-id rows: keep the higher of `local.relevance_score` and
 *     `cloud.similarity`.
 *   - Tx-id folding: when the local row carries a synthetic `local:`
 *     prefix (never anchored) and the cloud hit carries a real tx id,
 *     overwrite with the cloud value. Real local tx ids win against
 *     cloud rotations (Phase 1 decision; see decisions.md).
 *   - Cloud-only entries are projected into `SearchResult` shape
 *     (empty `content_hash` — the popup's Verify tab re-fetches on
 *     click) so the existing renderer doesn't need a discriminated
 *     union.
 *   - Cloud hits with an empty `attestation_id` are dropped (defensive
 *     against malformed server responses).
 *   - Final list is re-sorted by `relevance_score` descending and
 *     truncated to `limit`.
 */
export function mergeRecallHits(
  local: SearchResult[],
  cloud: CloudRecallHit[] | null,
  limit: number,
): SearchResult[] {
  if (!cloud || cloud.length === 0) {
    return local.slice(0, limit);
  }
  const byId = new Map<string, SearchResult>();
  for (const r of local) {
    byId.set(r.attestation_id, r);
  }
  for (const c of cloud) {
    if (!c.attestation_id) continue;
    const existing = byId.get(c.attestation_id);
    if (existing) {
      const better = c.similarity > existing.relevance_score;
      // Prefer cloud-side tx ids when local still carries synthetic
      // `local:` prefixes — the cloud row has been anchored.
      const solana_tx =
        existing.solana_tx.startsWith("local:") && c.solana_tx
          ? c.solana_tx
          : existing.solana_tx;
      const arweave_tx =
        existing.arweave_tx.startsWith("local:") && c.arweave_tx
          ? c.arweave_tx
          : existing.arweave_tx;
      byId.set(c.attestation_id, {
        ...existing,
        relevance_score: better ? c.similarity : existing.relevance_score,
        solana_tx,
        arweave_tx,
      });
    } else {
      byId.set(c.attestation_id, {
        attestation_id: c.attestation_id,
        content: c.content,
        // Cloud-only entries don't carry a content_hash today — render
        // with an empty hash; the popup's Verify tab re-fetches the
        // full row when the user clicks Open.
        content_hash: "",
        tags: c.tags ?? [],
        solana_tx: c.solana_tx ?? "",
        arweave_tx: c.arweave_tx ?? "",
        created_at: c.signed_at ?? "",
        relevance_score: c.similarity,
      });
    }
  }
  return Array.from(byId.values())
    .sort((a, b) => b.relevance_score - a.relevance_score)
    .slice(0, limit);
}
