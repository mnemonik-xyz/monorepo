// `mnemonic open <hash|link>` — decrypt and print a sealed memory.
//
// Reads the sealed blob from the server (by content hash) or parses a share
// link (URL with `#k=<base64url>` fragment) and decrypts it locally.
//
// Keychain: reads the private key when the hash form is used (needs the
// author's X25519 key, derived from the Ed25519 keypair).  For share links
// the key is embedded in the fragment — no identity key needed unless the
// link is missing the `#k=` fragment (then it falls back to the identity key).

import { fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions } from "../output.js";
import { openSession } from "../session.js";

export interface OpenOptions extends OutputOptions {
  baseUrl?: string;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

export async function runOpen(
  hashOrLink: string | undefined,
  opts: OpenOptions
): Promise<void> {
  if (!hashOrLink || hashOrLink.length === 0) {
    throw new UserError(
      "hash or link is required: `mnemonic open <hash|link>`"
    );
  }

  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;

  // open always reads the private key (to derive the X25519 decryption key).
  const { client, signer } = await openSession(baseUrl, opts, true);

  // Provide the keypair — for `open` we always need to decrypt.
  client.setKeypairProvider(() => signer.keypair());

  hint("opening sealed memory...", opts);
  let result;
  try {
    if (hashOrLink.startsWith("http://") || hashOrLink.startsWith("https://")) {
      // Share link — may use bearer key from fragment (importLink) or identity
      // key if the link contains no fragment.
      result = await client.importLink(hashOrLink);
    } else {
      // Content hash — decrypt using identity X25519 key.
      result = await client.openMemory(hashOrLink);
    }
  } catch (e) {
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => result.content);
}
