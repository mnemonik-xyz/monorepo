// `mnemonic prove [--challenge=<hex>]` — Decision 14: client-side.
//
// Reads identity.json. Decodes (or generates) a 32-byte challenge. Signs via
// `LocalSigner.sign(bytes)` (which calls WASM `sign_challenge` under the
// hood). Outputs `{pubkey, challenge_hex, signature_hex, did}`.
//
// No server call. The user can hand the printed JSON to a verifier who will
// run Ed25519 verify offline.

import { LocalSigner } from "@mnemonik-xyz/sdk";
import { randomBytes } from "node:crypto";

import { loadIdentity } from "../config.js";
import { fromSdkError, UserError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";

export interface ProveOptions extends OutputOptions {
  challenge?: string;
}

const DEFAULT_CHALLENGE_BYTES = 32;

export async function runProve(opts: ProveOptions): Promise<void> {
  const challenge = opts.challenge
    ? parseHex(opts.challenge)
    : new Uint8Array(randomBytes(DEFAULT_CHALLENGE_BYTES));

  if (challenge.length === 0) {
    throw new UserError("challenge must be at least 1 byte");
  }

  const kp = await loadIdentity();
  const signer = new LocalSigner(kp);

  let signature: Uint8Array;
  try {
    signature = await signer.sign(challenge);
  } catch (e) {
    throw fromSdkError(e);
  }

  const result = {
    pubkey: kp.pubkey,
    did: `did:sol:${kp.pubkey}`,
    challenge: bytesToHex(challenge),
    signature: bytesToHex(signature),
  };

  format(result, opts, (_d, _color) => {
    return [
      `pubkey:    ${result.pubkey}`,
      `did:       ${result.did}`,
      `challenge: ${result.challenge}`,
      `signature: ${result.signature}`,
    ].join("\n");
  });
}

function parseHex(s: string): Uint8Array {
  const trimmed = s.startsWith("0x") || s.startsWith("0X") ? s.slice(2) : s;
  if (trimmed.length === 0) {
    throw new UserError("--challenge: empty hex string");
  }
  if (trimmed.length % 2 !== 0) {
    throw new UserError("--challenge: hex string must have even length");
  }
  if (!/^[0-9a-fA-F]+$/.test(trimmed)) {
    throw new UserError("--challenge: contains non-hex characters");
  }
  const out = new Uint8Array(trimmed.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(trimmed.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function bytesToHex(b: Uint8Array): string {
  let s = "";
  for (let i = 0; i < b.length; i++) {
    s += b[i]!.toString(16).padStart(2, "0");
  }
  return s;
}
