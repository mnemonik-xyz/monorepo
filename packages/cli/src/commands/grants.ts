// `mnemonic grants` — list access grants the caller has created.
//
// Calls `GET /api/grants?reader=<pubkey>` and prints the result as a table.
// Uses the public key only for the query parameter; the private key is NOT
// read (no keychain access).

import { fromSdkError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";
import { openSession } from "../session.js";

export interface GrantsOptions extends OutputOptions {
  baseUrl?: string;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

export async function runGrants(opts: GrantsOptions): Promise<void> {
  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;

  // grants works from the public key alone — no keychain read.
  const { client } = await openSession(baseUrl, opts, false);

  let grants;
  try {
    grants = await client.listGrants();
  } catch (e) {
    throw fromSdkError(e);
  }

  format(grants, opts, (_d, _color) => {
    if (grants.length === 0) {
      return "no grants";
    }
    const lines = [`${grants.length} grant(s):`];
    for (const g of grants) {
      const reader = g.reader ? `  → ${g.reader}` : "  → (anonymous link)";
      lines.push(
        `  ${g.grantId.slice(0, 12)}  ${g.memoryHash.slice(0, 16)}…  ${g.createdAt}${reader}`
      );
    }
    return lines.join("\n");
  });
}
