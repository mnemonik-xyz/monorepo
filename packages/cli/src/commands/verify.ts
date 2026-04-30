// `mnemonic verify <attestation_id>` — calls `client.verify`.
//
// Exit-code mapping per task 5.md:
//   verified  → 0
//   tampered  → 3 (IntegrityError)
//   not_found → 1 (UserError)

import { LocalSigner, MnemonicClient } from "@mnemonik-xyz/sdk";

import { loadIdentity, loadToken } from "../config.js";
import { fromSdkError, IntegrityError, UserError } from "../errors.js";
import { format, type OutputOptions } from "../output.js";

export interface VerifyOptions extends OutputOptions {
  baseUrl?: string;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

export async function runVerify(
  attestationId: string,
  opts: VerifyOptions
): Promise<void> {
  if (!attestationId || attestationId.length === 0) {
    throw new UserError("attestation_id is required: `mnemonic verify <id>`");
  }

  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;
  const kp = await loadIdentity();
  const tok = loadToken();

  const client = new MnemonicClient({
    baseUrl,
    signer: new LocalSigner(kp),
    jwt: tok.jwt,
  });

  let result;
  try {
    result = await client.verify(attestationId);
  } catch (e) {
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    if (result.status === "verified") {
      const lines = [`status: verified`, `signer: ${result.signer}`];
      if (result.arweaveTx) lines.push(`arweave_tx: ${result.arweaveTx}`);
      if (result.solanaTx) lines.push(`solana_tx: ${result.solanaTx}`);
      return lines.join("\n");
    }
    if (result.status === "tampered") {
      return `status: tampered\nsigner: ${result.signer}\nreason: ${result.reason}`;
    }
    return `status: not_found`;
  });

  // Translate terminal status into exit code (the format() above emits
  // first so the user sees the result before the exit).
  if (result.status === "tampered") {
    throw new IntegrityError(
      `attestation ${attestationId} is tampered: ${result.reason}`
    );
  }
  if (result.status === "not_found") {
    throw new UserError(`attestation ${attestationId} not found`);
  }
}
