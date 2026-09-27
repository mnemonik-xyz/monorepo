// prepareFeedback and verifyFeedbackDocument — core Wave 3 deliverables.
//
// Signing message format (both proof types):
//   MNEMONIC_FEEDBACK_V1:0x<payload_hash lowercase hex>
//
// Two-level hash:
//   payload_hash  = keccak256(JCS(document.feedback))  ← Ed25519 signs this
//   feedbackHash  = keccak256(JCS(document))            ← goes on chain
//
// Verification uses @noble/ed25519 (verifyAsync) — no WASM, no COSE,
// reproducible by any third party.

import { verifyAsync as ed25519Verify } from "@noble/ed25519";
import { encodeGiveFeedback } from "./abi.js";
import { bytesToHex, hexToBytes, isValidChecksumAddress, keccak256Hex, toChecksumAddress } from "./evm.js";
import { jcsBytes, jcsStringify } from "./jcs.js";
import {
  GIVE_FEEDBACK_SELECTOR,
  GIVE_FEEDBACK_SIG,
  addressesForChain,
  CHAIN_IDS,
} from "./registry.js";
import type {
  Ed25519Proof,
  FeedbackPayload,
  MnemonicFeedbackV1,
  PrepareFeedbackInput,
  PrepareFeedbackOpts,
  PreparedFeedback,
  VerifyChecks,
  VerifyFeedbackInput,
  VerifyFeedbackResult,
} from "./types.js";

const enc = new TextEncoder();

// ── prepareFeedback ────────────────────────────────────────────────────────

export async function prepareFeedback(
  input: PrepareFeedbackInput,
  opts: PrepareFeedbackOpts
): Promise<PreparedFeedback> {
  const warnings: string[] = [];

  // ── Validate inputs ──────────────────────────────────────────────────────

  if (!isValidChecksumAddress(input.clientAddress)) {
    throw new FeedbackError(
      `clientAddress must be an EIP-55 checksum address, got: ${input.clientAddress}`
    );
  }

  const agentIdStr = String(BigInt(input.agentId));

  const valueNum = Number(input.value);
  if (!Number.isSafeInteger(valueNum)) {
    throw new FeedbackError(
      `value must be within the JSON safe-integer range (±${Number.MAX_SAFE_INTEGER})`
    );
  }

  if (
    !Number.isInteger(input.valueDecimals) ||
    input.valueDecimals < 0 ||
    input.valueDecimals > 18
  ) {
    throw new FeedbackError(`valueDecimals must be 0–18, got ${input.valueDecimals}`);
  }

  if (input.note !== undefined && input.note.length > 280) {
    throw new FeedbackError(`note must be ≤ 280 chars, got ${input.note.length}`);
  }

  // Warn (don't reject) on oversized tags — the contract accepts strings.
  for (const [k, v] of Object.entries(input.tags ?? {})) {
    if (v && v.length > 32) {
      warnings.push(`tag ${k} is longer than 32 chars — may fail the contract's tag comparisons`);
    }
  }

  if (opts.signer.pubkey !== input.mnemonic.ed25519_pubkey) {
    warnings.push(
      "signer.pubkey differs from mnemonic.ed25519_pubkey — the document will attribute the evidence to a key the signer does not control"
    );
  }

  // ── Resolve registry address ─────────────────────────────────────────────

  const chainId = input.chainId ?? CHAIN_IDS.ethereumMainnet;
  const addrs = addressesForChain(chainId);
  const reputationAddr = (input.registryAddress
    ? toChecksumAddress(input.registryAddress)
    : toChecksumAddress(addrs.reputationRegistry)) as `0x${string}`;

  // ── Build feedback payload ───────────────────────────────────────────────

  const feedbackPayload: FeedbackPayload = {
    agentRegistry: `eip155:${chainId}:${reputationAddr}`,
    agentId: agentIdStr,
    clientAddress: input.clientAddress,
    createdAt: input.createdAt,
    value: valueNum,
    valueDecimals: input.valueDecimals,
    mnemonic: input.mnemonic,
  };

  // Only include optional fields if present (never include null/undefined).
  if (input.tags?.tag1 !== undefined || input.tags?.tag2 !== undefined) {
    const tags: Record<string, string> = {};
    if (input.tags?.tag1 !== undefined) tags["tag1"] = input.tags.tag1;
    if (input.tags?.tag2 !== undefined) tags["tag2"] = input.tags.tag2;
    feedbackPayload.tags = tags;
  }
  if (input.endpoint !== undefined) feedbackPayload.endpoint = input.endpoint;
  if (input.note !== undefined) feedbackPayload.note = input.note;

  // ── payload_hash: keccak256(JCS(feedback)) ───────────────────────────────

  const payloadHash = keccak256Hex(jcsBytes(feedbackPayload)) as `0x${string}`;

  // ── Ed25519 sign ─────────────────────────────────────────────────────────

  const signedMessage = `MNEMONIC_FEEDBACK_V1:${payloadHash}`;
  const sigBytes = await opts.signer.sign(enc.encode(signedMessage));
  if (sigBytes.length !== 64) {
    throw new FeedbackError(
      `signer returned ${sigBytes.length} bytes — Ed25519 signature must be 64 bytes`
    );
  }

  const ed25519Proof: Ed25519Proof = {
    type: "ed25519",
    kid: opts.signer.pubkey,
    sig: uint8ArrayToBase64(sigBytes),
  };

  const proofs = opts.eip191Proof
    ? [ed25519Proof, opts.eip191Proof]
    : [ed25519Proof];

  // ── Assemble document ────────────────────────────────────────────────────

  const document: MnemonicFeedbackV1 = {
    schema: "MNEMONIC_FEEDBACK_V1",
    feedback: feedbackPayload,
    payload_hash: payloadHash,
    proofs,
  };

  // ── feedbackHash: keccak256(JCS(document)) ───────────────────────────────

  const feedbackHash = keccak256Hex(jcsBytes(document)) as `0x${string}`;
  const documentJson = jcsStringify(document);

  // ── Encode giveFeedback calldata ─────────────────────────────────────────

  const calldata = encodeGiveFeedback({
    agentId: BigInt(agentIdStr),
    value: BigInt(valueNum),
    valueDecimals: input.valueDecimals,
    tag1: input.tags?.tag1 ?? "",
    tag2: input.tags?.tag2 ?? "",
    endpoint: input.endpoint ?? "",
    feedbackURI: input.feedbackUri,
    feedbackHash: hexToBytes(feedbackHash),
  });

  return {
    document,
    documentJson,
    payloadHash,
    feedbackHash,
    feedbackUri: input.feedbackUri,
    onchain: {
      chainId,
      to: reputationAddr,
      data: `0x${bytesToHex(calldata)}` as `0x${string}`,
      value: "0x0",
      functionSignature: GIVE_FEEDBACK_SIG,
      selector: GIVE_FEEDBACK_SELECTOR,
    },
    preflight: {
      requiredSender: input.clientAddress as `0x${string}`,
      chainId,
    },
    warnings,
  };
}

// ── verifyFeedbackDocument ─────────────────────────────────────────────────

export async function verifyFeedbackDocument(
  input: VerifyFeedbackInput
): Promise<VerifyFeedbackResult> {
  const checks: VerifyChecks = {
    feedbackHash: false,
    payloadHash: false,
    ed25519: false,
    senderBinding: "not-checked",
  };

  let document: MnemonicFeedbackV1;
  try {
    const parsed = JSON.parse(input.documentJson) as unknown;
    if (!isValidDocumentShape(parsed)) {
      return { valid: false, senderBinding: "not-checked", checks, error: "document is not a valid MNEMONIC_FEEDBACK_V1 shape" };
    }
    document = parsed;
  } catch (e) {
    return {
      valid: false,
      senderBinding: "not-checked",
      checks,
      error: `JSON.parse failed: ${e instanceof Error ? e.message : String(e)}`,
    };
  }

  try {
    // ── Check 1: feedbackHash = keccak256(JCS(document)) ──────────────────
    const recomputedFeedbackHash = keccak256Hex(jcsBytes(document));
    if (input.onchainFeedbackHash) {
      checks.feedbackHash =
        recomputedFeedbackHash.toLowerCase() ===
        input.onchainFeedbackHash.toLowerCase();
    } else {
      // No on-chain hash to compare against — mark as unchecked but pass.
      checks.feedbackHash = true;
    }

    // ── Check 2: payload_hash = keccak256(JCS(feedback)) ──────────────────
    const recomputedPayloadHash = keccak256Hex(jcsBytes(document.feedback));
    checks.payloadHash =
      recomputedPayloadHash.toLowerCase() === document.payload_hash.toLowerCase();

    // ── Check 3: Ed25519 signature ─────────────────────────────────────────
    const ed25519Proof = document.proofs.find((p) => p.type === "ed25519") as
      | Ed25519Proof
      | undefined;

    if (ed25519Proof) {
      const signedMessage = `MNEMONIC_FEEDBACK_V1:${document.payload_hash.toLowerCase()}`;
      const msgBytes = enc.encode(signedMessage);
      const sigBytes = base64ToUint8Array(ed25519Proof.sig);
      const pubkeyBytes = bs58Decode(ed25519Proof.kid);

      checks.ed25519 = await ed25519Verify(sigBytes, msgBytes, pubkeyBytes).catch(() => false);
    }

    // ── Check 4: sender binding (optional) ────────────────────────────────
    if (input.onchainSender) {
      const senderNorm = input.onchainSender.toLowerCase();
      const clientNorm = document.feedback.clientAddress.toLowerCase();
      checks.senderBinding = senderNorm === clientNorm ? "verified" : "mismatch";
    }
  } catch (e) {
    return {
      valid: false,
      senderBinding: checks.senderBinding,
      checks,
      error: `verification error: ${e instanceof Error ? e.message : String(e)}`,
    };
  }

  const valid =
    checks.feedbackHash &&
    checks.payloadHash &&
    checks.ed25519 &&
    checks.senderBinding !== "mismatch";

  return { valid, senderBinding: checks.senderBinding, checks };
}

// ── Helpers ────────────────────────────────────────────────────────────────

function isValidDocumentShape(v: unknown): v is MnemonicFeedbackV1 {
  if (!v || typeof v !== "object") return false;
  const d = v as Record<string, unknown>;
  return (
    d["schema"] === "MNEMONIC_FEEDBACK_V1" &&
    typeof d["feedback"] === "object" &&
    d["feedback"] !== null &&
    typeof d["payload_hash"] === "string" &&
    Array.isArray(d["proofs"])
  );
}

function uint8ArrayToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]!);
  }
  return btoa(binary);
}

function base64ToUint8Array(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

const BS58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function bs58Decode(s: string): Uint8Array {
  let n = 0n;
  for (const c of s) {
    const d = BS58_ALPHABET.indexOf(c);
    if (d < 0) throw new FeedbackError(`base58: invalid character '${c}'`);
    n = n * 58n + BigInt(d);
  }
  const bytes: number[] = [];
  while (n > 0n) {
    bytes.unshift(Number(n & 0xffn));
    n >>= 8n;
  }
  let leading = 0;
  for (const c of s) {
    if (c === "1") leading++;
    else break;
  }
  return new Uint8Array([...new Array(leading).fill(0), ...bytes]);
}

export class FeedbackError extends Error {
  override name = "FeedbackError";
}
