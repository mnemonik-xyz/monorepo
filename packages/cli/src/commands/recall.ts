// `mnemonic recall <query> [--top-k=N] [--tag=foo] [--sealed]`
//
// Reads identity + token, calls `client.recall(query, {topK, tags})`, prints
// the hits. Default top-k = 5; `--tag` is shorthand for a single-element
// `tags` filter.
//
// With `--sealed`: also calls `client.recallSealed(query, {topK})` and merges
// the results.  The sealed recall embeds the query locally and ranks sealed
// memories by cosine similarity — the server never sees the plaintext query.
// This reads the private key (once) to open sealed memories and rebuild the
// local index.
//
// Works from the public key alone (without --sealed): never reads the private
// key or the OS keychain. An expired token is renewed silently only when that
// needs no keychain access (see `session.ts`); otherwise a clear hint is
// printed rather than a silent anonymous (public-pool) recall.

import { fromSdkError, UserError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";
import { openSession } from "../session.js";

export interface RecallOptions extends OutputOptions {
  topK?: number;
  tag?: string;
  baseUrl?: string;
  /** `--sealed`: also recall sealed memories, merging with server results. */
  sealed?: boolean;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";
const DEFAULT_TOP_K = 5;

export async function runRecall(
  query: string,
  opts: RecallOptions
): Promise<void> {
  if (!query || query.length === 0) {
    throw new UserError("query is required: `mnemonic recall <query>`");
  }

  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const topK = typeof opts.topK === "number" ? opts.topK : DEFAULT_TOP_K;
  const withSealed = opts.sealed === true;

  // Pre-flight (identity/JWT mismatch) runs inside, BEFORE any fetch.
  // For sealed recall we need the keypair; otherwise public key only.
  const { client, signer } = await openSession(baseUrl, opts, withSealed);

  if (withSealed) {
    // Provide keypair for sealed index recall.
    client.setKeypairProvider(() => signer.keypair());
  }

  let result;
  try {
    result = await client.recall(query, {
      topK,
      ...(opts.tag ? { tags: [opts.tag] } : {}),
    });
  } catch (e) {
    throw fromSdkError(e);
  }

  // Optionally merge sealed recall results.
  type SealedRow = { memoryHash: string; similarity: number };
  let sealedHits: SealedRow[] = [];
  if (withSealed) {
    try {
      sealedHits = await client.recallSealed(query, { topK });
    } catch (e) {
      throw fromSdkError(e);
    }
  }

  format({ result, sealedHits }, opts, (_d, _color) => {
    const lines: string[] = [];
    if (result.hits.length > 0 || sealedHits.length === 0) {
      if (result.hits.length === 0) {
        lines.push(`no hits (queried ${result.total} memories)`);
      } else {
        lines.push(`${result.hits.length} hit(s) of ${result.total}:`);
        for (const h of result.hits) {
          lines.push(
            `  ${h.attestationId.slice(0, 12)}  sim=${h.similarity.toFixed(3)}  ` +
              (h.tags && h.tags.length > 0 ? `[${h.tags.join(",")}]  ` : "") +
              `${truncate(h.content, 80)}`
          );
        }
      }
    }
    if (sealedHits.length > 0) {
      lines.push(`${sealedHits.length} sealed hit(s):`);
      for (const h of sealedHits) {
        lines.push(
          `  ${h.memoryHash.slice(0, 16)}  sim=${h.similarity.toFixed(3)}`
        );
      }
    }
    return lines.join("\n") || `no hits (queried ${result.total} memories)`;
  });
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return `${s.slice(0, n - 1)}…`;
}
