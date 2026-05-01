// `mnemonic init [--force]` — generate a fresh keypair and save it to
// ~/.mnemonic/identity.json (mode 0600).
//
// Refuses to overwrite an existing identity unless `--force` is passed.
// Surfaces the new pubkey + DID so users can verify the file matches what
// the server will see.

import { unlinkSync } from "node:fs";

import { Keypair } from "@mnemonik-xyz/sdk";

import {
  identityExists,
  identityPath,
  loadToken,
  saveIdentity,
  tokenExists,
  tokenPath,
} from "../config.js";
import { CliError, fromSdkError, UserError } from "../errors.js";
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

  // 0.1.5: a stale token.json (logged-in JWT.sub ≠ new keypair.pubkey) would
  // immediately trip the pre-flight check on the next sign/recall/verify.
  // Auto-clear it here and emit a warning so the user knows what changed.
  // If the token's sub already matches the new pubkey (rare — re-init of the
  // same keypair), keep it.
  if (tokenExists()) {
    let staleSub: string | null = null;
    try {
      const tok = loadToken();
      if (tok.sub !== kp.pubkey) staleSub = tok.sub;
    } catch (e) {
      // Token unreadable or expired — treat as stale and clear it. Bubble
      // unexpected (non-CliError) failures up.
      if (!(e instanceof CliError)) throw e;
      staleSub = "(unreadable)";
    }
    if (staleSub !== null) {
      try {
        unlinkSync(tokenPath());
      } catch {
        /* best-effort — ENOENT is fine */
      }
      process.stderr.write(
        `warning: cleared stale logged-in JWT (sub ${staleSub} != new pubkey ${kp.pubkey})\n` +
          `         re-run \`mnemonic login\` to authenticate as new identity.\n`
      );
    }
  }

  const data = {
    pubkey: kp.pubkey,
    did: `did:sol:${kp.pubkey}`,
    path: identityPath(),
  };
  format(data, opts, () => {
    const lines = [
      `identity created: ${identityPath()}`,
      `pubkey: ${kp.pubkey}`,
      `did:    did:sol:${kp.pubkey}`,
    ];
    return lines.join("\n");
  });
  hint(
    "tip: used Mnemonic in a browser? `mnemonic identity import --ticket <uuid>` " +
      "round-trips your existing keypair instead of replacing it.",
    opts
  );
}
