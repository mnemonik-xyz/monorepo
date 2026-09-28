// `mnemonic share <hash> --to <did|key> | --link` — grant access to a sealed memory.
//
// Two modes:
//   --link          → anonymous bearer link (URL with `#k=<base64url>` fragment)
//   --to <did|key>  → targeted grant for a specific reader DID or X25519 key
//
// Reads the private key to derive the X25519 key needed to re-wrap the
// content encryption key `K` for the recipient.

import { fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions } from "../output.js";
import { openSession } from "../session.js";

export interface ShareOptions extends OutputOptions {
  to?: string;
  link?: boolean;
  baseUrl?: string;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

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

  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;

  // share always reads the private key.
  const { client, signer } = await openSession(baseUrl, opts, true);
  client.setKeypairProvider(() => signer.keypair());

  hint(isLink ? "creating share link..." : `creating grant for ${to}...`, opts);
  let result;
  try {
    if (isLink) {
      result = await client.share(hash, "link");
    } else {
      // Parse the --to value: treat as a DID / kid, and use a zero X25519 key
      // placeholder (the server will use the reader's registered key).
      // For targeted grants the server looks up the reader's X25519 key.
      const readerKid = to!;
      // Use a placeholder x25519Pub — the server is authoritative for the
      // reader's registered key; the CLI passes kid only.
      const x25519Pub = new Uint8Array(32);
      result = await client.share(hash, { kid: readerKid, x25519Pub });
    }
  } catch (e) {
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    if (result.type === "link") {
      return `url: ${result.url}`;
    }
    // Targeted grant: show the CBOR bytes as hex.
    const hex = Array.from(result.grantCbor)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
    return `grant_cbor: ${hex}`;
  });
}
