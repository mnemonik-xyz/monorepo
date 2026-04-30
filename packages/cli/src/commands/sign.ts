// `mnemonic sign [content]` — read identity + token, instantiate a
// MnemonicClient, run the pending-bundle / sign-callback flow.
//
// Content sources:
//   1. positional argument (preferred)
//   2. stdin if argument is missing AND stdin is not a TTY
//   3. otherwise: UserError "no content provided"
//
// Tags from `--tags=a,b,c` (comma-separated, trimmed, empty entries dropped).

import { LocalSigner, MnemonicClient } from "@mnemonik-xyz/sdk";

import { loadIdentity, loadToken } from "../config.js";
import { fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions } from "../output.js";

export interface SignOptions extends OutputOptions {
  tags?: string;
  baseUrl?: string;
  /** Internal — read content from this string instead of stdin (tests). */
  content?: string;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

export async function runSign(
  positional: string | undefined,
  opts: SignOptions
): Promise<void> {
  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;

  const content = await resolveContent(positional, opts);
  if (!content) {
    throw new UserError(
      "no content provided; pass as a positional argument or pipe via stdin"
    );
  }

  const tags = parseTags(opts.tags);
  const kp = await loadIdentity();
  const tok = loadToken();

  const client = new MnemonicClient({
    baseUrl,
    signer: new LocalSigner(kp),
    jwt: tok.jwt,
  });
  client.setKeypair(kp);

  hint("signing memory...", opts);
  let result;
  try {
    result = await client.signMemory(content, tags.length > 0 ? { tags } : {});
  } catch (e) {
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    const lines = [
      `attestation_id: ${result.attestationId}`,
      `signed_at:      ${result.signedAt}`,
      `status:         ${result.status}`,
    ];
    if (result.contentHash) lines.push(`content_hash:   ${result.contentHash}`);
    if (result.arweaveTx) lines.push(`arweave_tx:     ${result.arweaveTx}`);
    if (result.solanaTx) lines.push(`solana_tx:      ${result.solanaTx}`);
    return lines.join("\n");
  });
}

async function resolveContent(
  positional: string | undefined,
  opts: SignOptions
): Promise<string> {
  if (positional && positional.length > 0) return positional;
  if (typeof opts.content === "string") return opts.content;
  // stdin only if it is piped (not a TTY).
  if (process.stdin.isTTY) return "";
  return readStdin();
}

function readStdin(): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => {
      data += chunk;
    });
    process.stdin.on("end", () => resolve(data.replace(/\r?\n$/, "")));
    process.stdin.on("error", reject);
  });
}

function parseTags(raw: string | undefined): string[] {
  if (!raw) return [];
  return raw
    .split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}
