// `mnemonic init [--force]` — generate a fresh keypair and save it to
// ~/.mnemonic/identity.json (mode 0600).
//
// Refuses to overwrite an existing identity unless `--force` is passed.
// Surfaces the new pubkey + DID so users can verify the file matches what
// the server will see.

import { Keypair } from "@mnemonik-xyz/sdk";

import { identityExists, identityPath, saveIdentity } from "../config.js";
import { fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions } from "../output.js";

export interface InitOptions extends OutputOptions {
  force?: boolean;
}

export async function runInit(opts: InitOptions): Promise<void> {
  if (identityExists() && !opts.force) {
    throw new UserError(
      `identity already exists at ${identityPath()}; pass --force to overwrite ` +
        `(this will replace your keypair — use \`mnemonic identity export\` first)`
    );
  }

  let kp: Keypair;
  try {
    kp = await Keypair.generate();
  } catch (e) {
    throw fromSdkError(e);
  }
  saveIdentity(kp);

  const data = {
    pubkey: kp.pubkey,
    did: `did:sol:${kp.pubkey}`,
    path: identityPath(),
  };
  format(data, opts, (_d, color) => {
    const lines = [
      `identity created: ${identityPath()}`,
      `pubkey: ${kp.pubkey}`,
      `did:    did:sol:${kp.pubkey}`,
    ];
    return lines.map((l) => (color ? l : l)).join("\n");
  });
  hint(
    "tip: used Mnemonic in a browser? `mnemonic identity import --ticket <uuid>` " +
      "round-trips your existing keypair instead of replacing it.",
    opts
  );
}
