// `mnemonic erc8004 feedback` and `mnemonic erc8004 feedback-verify`
//
// feedback     — offline: builds MNEMONIC_FEEDBACK_V1, signs with local
//                identity, outputs calldata + document JSON.
// feedback-verify — offline: verifies hashes + Ed25519 proof; optionally
//                   checks on-chain sender binding.
//
// Neither command makes a Mnemonic server call. The private key is read from
// the local keystore only by `feedback` (same as `mnemonic prove`).

import { readFileSync, writeFileSync } from "node:fs";
import {
  LocalSigner,
  prepareFeedback,
  verifyFeedbackDocument,
  checkSelfPromotion,
} from "@mnemonik-xyz/sdk";

import { loadIdentity } from "../config.js";
import { UserError } from "../errors.js";
import { colors, format, hint, warn, type OutputOptions } from "../output.js";

// ── erc8004 feedback ───────────────────────────────────────────────────────

export interface FeedbackOptions extends OutputOptions {
  agentId: string;
  value: string;
  valueDecimals?: number;
  clientAddress: string;
  feedbackUri: string;
  attestationId: string;
  blake3: string;
  tag1?: string;
  tag2?: string;
  endpoint?: string;
  note?: string;
  createdAt?: string;
  chainId?: number;
  registry?: string;
  rpcUrl?: string;
  out?: string;
}

export async function runErc8004Feedback(opts: FeedbackOptions): Promise<void> {
  const kp = await loadIdentity();
  const signer = new LocalSigner(kp);

  const createdAt =
    opts.createdAt ?? new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

  // Self-promotion guard (pre-flight).
  if (opts.rpcUrl || !opts.rpcUrl) {
    const guard = await checkSelfPromotion({
      agentId: opts.agentId,
      clientAddress: opts.clientAddress,
      chainId: opts.chainId,
      rpcUrl: opts.rpcUrl,
    });
    for (const w of guard.warnings) warn(w, opts);
    if (guard.status === "self" || guard.status === "operator" || guard.status === "approved") {
      throw new UserError(
        `self-promotion guard: clientAddress is the agent's ${guard.status} — ` +
          "the registry will reject this transaction"
      );
    }
  }

  hint("building feedback document...", opts);

  const result = await prepareFeedback(
    {
      agentId: opts.agentId,
      value: parseInt(opts.value, 10),
      valueDecimals: opts.valueDecimals ?? 2,
      clientAddress: opts.clientAddress,
      feedbackUri: opts.feedbackUri,
      mnemonic: {
        schema: "MEMORY_V1",
        attestation_id: opts.attestationId,
        blake3: opts.blake3,
        ed25519_pubkey: kp.pubkey,
      },
      ...(opts.tag1 !== undefined || opts.tag2 !== undefined
        ? {
            tags: {
              ...(opts.tag1 !== undefined ? { tag1: opts.tag1 } : {}),
              ...(opts.tag2 !== undefined ? { tag2: opts.tag2 } : {}),
            },
          }
        : {}),
      ...(opts.endpoint !== undefined ? { endpoint: opts.endpoint } : {}),
      ...(opts.note !== undefined ? { note: opts.note } : {}),
      createdAt,
      ...(opts.chainId !== undefined ? { chainId: opts.chainId } : {}),
      ...(opts.registry !== undefined ? { registryAddress: opts.registry } : {}),
    },
    { signer }
  );

  for (const w of result.warnings) warn(w, opts);

  if (opts.out) {
    writeFileSync(opts.out, result.documentJson, "utf8");
    hint(`document written to ${opts.out}`, opts);
  }

  format(result, opts, (_d, color) => {
    const lines = [
      `feedback_hash:  ${colors.cyan(result.feedbackHash, color)}`,
      `payload_hash:   ${result.payloadHash}`,
      `chain_id:       ${result.onchain.chainId}`,
      `registry:       ${result.onchain.to}`,
      `selector:       ${result.onchain.selector}`,
      `calldata:       ${result.onchain.data}`,
      `required_sender: ${result.preflight.requiredSender}`,
    ];
    if (!opts.out) {
      lines.push(``, `document_json:`, result.documentJson);
    } else {
      lines.push(`document:       ${opts.out}`);
    }
    if (result.warnings.length > 0) {
      lines.push(``, `warnings:`);
      for (const w of result.warnings) lines.push(`  - ${w}`);
    }
    return lines.join("\n");
  });
}

// ── erc8004 feedback-verify ────────────────────────────────────────────────

export interface FeedbackVerifyOptions extends OutputOptions {
  file?: string;
  feedbackHash?: string;
  sender?: string;
}

export async function runErc8004FeedbackVerify(
  opts: FeedbackVerifyOptions
): Promise<void> {
  let documentJson: string;
  if (opts.file) {
    documentJson = readFileSync(opts.file, "utf8");
  } else if (!process.stdin.isTTY) {
    documentJson = await readStdin();
  } else {
    throw new UserError(
      "provide a document via --file or pipe it via stdin"
    );
  }

  const result = await verifyFeedbackDocument({
    documentJson,
    ...(opts.feedbackHash !== undefined
      ? { onchainFeedbackHash: opts.feedbackHash }
      : {}),
    ...(opts.sender !== undefined ? { onchainSender: opts.sender } : {}),
  });

  if (result.error) {
    throw new UserError(`verification failed: ${result.error}`);
  }

  format(result, opts, (_d, color) => {
    const ok = (v: boolean) => (v ? colors.green("✓", color) : colors.red("✗", color));
    const lines = [
      `valid:          ${result.valid ? colors.green("yes", color) : colors.red("NO", color)}`,
      `feed_hash:      ${ok(result.checks.feedbackHash)} ${result.checks.feedbackHash ? "match" : "MISMATCH"}`,
      `payload_hash:   ${ok(result.checks.payloadHash)} ${result.checks.payloadHash ? "match" : "MISMATCH"}`,
      `ed25519:        ${ok(result.checks.ed25519)} ${result.checks.ed25519 ? "valid" : "INVALID"}`,
      `sender_binding: ${result.checks.senderBinding}`,
    ];
    return lines.join("\n");
  });

  if (!result.valid) process.exit(3);
}

function readStdin(): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (c) => { data += c; });
    process.stdin.on("end", () => resolve(data));
    process.stdin.on("error", reject);
  });
}
