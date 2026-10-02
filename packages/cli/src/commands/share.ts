// `mnemonic share <hash> --to <did|key> | --link` — grant access to a sealed memory.
//
// This legacy command reports a migration error before HTTP or key access.
import { UserError } from "../errors.js";
import { type OutputOptions } from "../output.js";

export interface ShareOptions extends OutputOptions {
  to?: string;
  link?: boolean;
  baseUrl?: string;
}

export async function runShare(
  hash: string | undefined,
  opts: ShareOptions
): Promise<void> {
  if (!hash || hash.length === 0) {
    throw new UserError("memory hash is required: `mnemonic share <hash>`");
  }

  const isLink = opts.link === true;
  const to = opts.to;

  if (!isLink && !to) {
    throw new UserError(
      "one of --link or --to <did|key> is required: " +
        "`mnemonic share <hash> --link` or `mnemonic share <hash> --to <did|key>`"
    );
  }

  if (isLink && to) {
    throw new UserError(
      "--link and --to are mutually exclusive"
    );
  }

  throw new UserError("hosted grant creation is retired; distribute signed grants client-side or use sealed A2A recipient grants. Existing grants remain readable with mnemonic grants.");
}
