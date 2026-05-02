// `mnemonic init` — set up a CLI identity at ~/.mnemonic/identity.json.
//
// 0.1.6 BREAKING CHANGE: default mode requires an explicit flag.
// Pre-0.1.6, `mnemonic init` (no flags) generated a fresh keypair. That
// silently drifted vs. the user's webapp localStorage keypair — causing
// "pending bundle owner mismatch" 403s on every browser-mediated sign.
//
// New mode matrix:
//
//   mnemonic init                     → ERROR: pick a mode (helpful message)
//   mnemonic init --ticket <uuid>     → import from webapp via Send-to-CLI ticket
//                                       (RECOMMENDED — keeps CLI + webapp keypairs in sync)
//   mnemonic init --standalone        → generate a fresh, standalone CLI-only
//                                       keypair (current pre-0.1.6 behavior;
//                                       opt-in for advanced users who do not
//                                       use the webapp at all)
//   mnemonic init --force ...         → overwrite existing identity (still
//                                       requires --ticket OR --standalone)

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
import { runIdentityImport } from "./identity.js";

export interface InitOptions extends OutputOptions {
  force?: boolean;
  /** Opt-in to fresh keypair generation (pre-0.1.6 default behavior). */
  standalone?: boolean;
  /** Shortcut: redeem a webapp Send-to-CLI ticket via `runIdentityImport`. */
  ticket?: string;
  /** Server base URL override, forwarded to ticket redemption. */
  baseUrl?: string;
}

const PICK_A_MODE_MSG = [
  "mnemonic init: pick a mode.",
  "",
  "  1. Pair with your webapp identity  (RECOMMENDED — avoids keypair drift)",
  "       a. Open https://mnemonik.xyz/install in your browser",
  "       b. Click 'Send to CLI' to get a one-time ticket UUID",
  "       c. Run:  mnemonic init --ticket <uuid>",
  "",
  "  2. Generate a standalone CLI-only keypair  (advanced)",
  "       Run:  mnemonic init --standalone",
  "",
  "       Note: a standalone keypair will NOT match your webapp's",
  "       localStorage keypair. Browser-mediated sign flows in Cursor /",
  "       VS Code / Claude.ai will fail with 'pending bundle owner",
  "       mismatch' until you align them via 'mnemonic identity import'.",
  "",
  "  3. Import an existing keypair JSON file",
  "       Run:  mnemonic identity import --file <path>",
].join("\n");

export async function runInit(opts: InitOptions): Promise<void> {
  // Mutually exclusive modes.
  if (opts.ticket && opts.standalone) {
    throw new UserError(
      "mnemonic init: --ticket and --standalone are mutually exclusive"
    );
  }

  // Mode 1 — paired with webapp via Send-to-CLI ticket.
  if (opts.ticket) {
    // Delegate to the same code path as `mnemonic identity import --ticket`
    // so behavior + error surfaces stay consistent.
    await runIdentityImport({
      ...opts,
      ticket: opts.ticket,
      ...(opts.force !== undefined ? { force: opts.force } : {}),
      ...(opts.baseUrl !== undefined ? { baseUrl: opts.baseUrl } : {}),
    });
    // Auto-clear stale token (mirrors --standalone branch's behavior).
    clearStaleTokenIfAny(opts);
    return;
  }

  // Mode 2 — standalone fresh keypair (pre-0.1.6 default).
  if (opts.standalone) {
    return runStandalone(opts);
  }

  // Default — no mode flag → helpful error.
  throw new UserError(PICK_A_MODE_MSG);
}

async function runStandalone(opts: InitOptions): Promise<void> {
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

  clearStaleTokenIfAny(opts, kp.pubkey);

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
    "tip: standalone keypair generated. If you ALSO use the webapp, run " +
      "`mnemonic identity import --ticket <uuid>` to align them and avoid " +
      "'pending bundle owner mismatch' on browser-mediated signs.",
    opts
  );
}

/**
 * 0.1.5+: clear a stale token.json (logged-in JWT.sub ≠ identity.pubkey).
 * Without this, the next sign/recall/verify trips the pre-flight check.
 *
 * Pulled out of `runStandalone` so the ticket-import branch (`runInit` →
 * `runIdentityImport`) can also call it. When `expectedPubkey` is null we
 * load the new identity from disk to read it.
 */
function clearStaleTokenIfAny(
  opts: OutputOptions,
  expectedPubkey: string | null = null
): void {
  if (!tokenExists()) return;

  let staleSub: string | null = null;
  try {
    const tok = loadToken();
    if (expectedPubkey !== null && tok.sub !== expectedPubkey) {
      staleSub = tok.sub;
    } else if (expectedPubkey === null) {
      // No expected pubkey passed in — read from the just-imported identity.
      // Defer the require to avoid a circular import at module-load time.
      // eslint-disable-next-line @typescript-eslint/no-require-imports
      const { loadIdentityJson } = require("../config.js");
      const id = loadIdentityJson();
      if (tok.sub !== id.pubkey_base58) staleSub = tok.sub;
    }
  } catch (e) {
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
      `warning: cleared stale logged-in JWT (sub ${staleSub} did not match new identity pubkey)\n` +
        `         re-run \`mnemonic login\` to authenticate as new identity.\n`
    );
  }
  void opts;
}
