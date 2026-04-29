// `mnemonic recall <query> [--top-k=N] [--tag=foo]`
//
// Reads identity + token, calls `client.recall(query, {topK, tags})`, prints
// the hits. Default top-k = 5; `--tag` is shorthand for a single-element
// `tags` filter.

import { LocalSigner, MnemonicClient } from "@mnemonik-xyz/sdk";

import { loadIdentity, loadToken } from "../config.js";
import { fromSdkError, UserError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";

export interface RecallOptions extends OutputOptions {
  topK?: number;
  tag?: string;
  baseUrl?: string;
}

const DEFAULT_BASE_URL = "https://mc.mnemonik.xyz";
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
  const kp = await loadIdentity();
  const tok = loadToken();

  const client = new MnemonicClient({
    baseUrl,
    signer: new LocalSigner(kp),
    jwt: tok.jwt,
  });

  let result;
  try {
    result = await client.recall(query, {
      topK,
      ...(opts.tag ? { tags: [opts.tag] } : {}),
    });
  } catch (e) {
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    if (result.hits.length === 0) {
      return `no hits (queried ${result.total} memories)`;
    }
    const lines = [`${result.hits.length} hit(s) of ${result.total}:`];
    for (const h of result.hits) {
      lines.push(
        `  ${h.attestationId.slice(0, 12)}  sim=${h.similarity.toFixed(3)}  ` +
          (h.tags && h.tags.length > 0 ? `[${h.tags.join(",")}]  ` : "") +
          `${truncate(h.content, 80)}`
      );
    }
    return lines.join("\n");
  });
}

function truncate(s: string, n: number): string {
  if (s.length <= n) return s;
  return `${s.slice(0, n - 1)}…`;
}
