// `mnemonic recall <query> [--top-k=N] [--tag=foo] [--sealed]`
//
// Reads identity + token, calls `client.recall(query, {topK, tags})`, prints
// the hits. Default top-k = 5; `--tag` is shorthand for a single-element
// `tags` filter.
//
// Sealed recall requires caller-local indexing and an explicit local embedder.
// This CLI does not provide those facilities; reject before disclosing the query.

import { fromSdkError, UserError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";
import { openSession } from "../session.js";

export interface RecallOptions extends OutputOptions {
  topK?: number;
  tag?: string;
  baseUrl?: string;
  /** Legacy flag; fails before network until a caller-local index/embedder is provided. */
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

  if (opts.sealed) throw new UserError("CLI sealed recall is unavailable: use SDK recallSealed with a caller-local embedder and locally prepared memories. No query was sent.");

  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const topK = typeof opts.topK === "number" ? opts.topK : DEFAULT_TOP_K;
  const { client } = await openSession(baseUrl, opts, false);

  let result;
  try {
    result = await client.recall(query, {
      topK,
      ...(opts.tag ? { tags: [opts.tag] } : {}),
    });
  } catch (e) {
    throw fromSdkError(e);
  }

  const sealedHits: Array<{memoryHash:string;similarity:number}> = [];

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
