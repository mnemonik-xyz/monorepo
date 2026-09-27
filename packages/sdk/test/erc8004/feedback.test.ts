// prepareFeedback and verifyFeedbackDocument tests.
//
// Uses a deterministic mock signer (fixed private key via @noble/ed25519) so
// the tests are hermetic and the fixture is reproducible without Mnemonic code.

import { signAsync, getPublicKeyAsync } from "@noble/ed25519";
import { describe, expect, it } from "vitest";
import { prepareFeedback, verifyFeedbackDocument, FeedbackError } from "../../src/erc8004/feedback.js";
import { bytesToHex } from "../../src/erc8004/evm.js";

// ── Deterministic test keypair ─────────────────────────────────────────────
//
// Private key: 32 bytes of 0x42.
// Public key: derived via @noble/ed25519 (32 bytes, then base58-encoded).
//
// To reproduce the pubkey outside this test:
//   node -e "
//     const { getPublicKeyAsync } = await import('@noble/ed25519');
//     const pub = await getPublicKeyAsync(new Uint8Array(32).fill(0x42));
//     const b = n => { let v=0n; for(const c of s)v=v*58n+BigInt('123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'.indexOf(c)); ... }
//   "

const FIXED_PRIVKEY = new Uint8Array(32).fill(0x42);

// Minimal bs58 encoder for test setup
function bs58Encode(bytes: Uint8Array): string {
  const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let n = 0n;
  for (const b of bytes) n = n * 256n + BigInt(b);
  let out = "";
  while (n > 0n) { out = ALPHABET[Number(n % 58n)]! + out; n /= 58n; }
  for (const b of bytes) { if (b !== 0) break; out = "1" + out; }
  return out;
}

async function makeSigner() {
  const pubkeyBytes = await getPublicKeyAsync(FIXED_PRIVKEY);
  const pubkey = bs58Encode(pubkeyBytes);
  return {
    pubkey,
    sign: async (bytes: Uint8Array) => signAsync(bytes, FIXED_PRIVKEY),
    pubkeyBytes,
  };
}

// ── Fixture input ──────────────────────────────────────────────────────────

function fixtureInput(pubkey: string) {
  return {
    agentId: "42",
    value: 9800,
    valueDecimals: 2,
    clientAddress: "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",
    feedbackUri: "https://example.com/feedback/1.json",
    mnemonic: {
      schema: "MEMORY_V1",
      attestation_id: "mn_01jtest",
      blake3: "a".repeat(64),
      ed25519_pubkey: pubkey,
      anchor: { chain: "solana", kind: "spl-memo", ref: "5sig" },
    },
    tags: { tag1: "quality", tag2: "research" },
    createdAt: "2026-09-27T12:00:00Z",
    chainId: 1,
  } as const;
}

// ── prepareFeedback ────────────────────────────────────────────────────────

describe("prepareFeedback", () => {
  it("returns PreparedFeedback with correct structure", async () => {
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    expect(result.document.schema).toBe("MNEMONIC_FEEDBACK_V1");
    expect(result.document.feedback.agentId).toBe("42");
    expect(result.document.feedback.value).toBe(9800);
    expect(result.document.feedback.clientAddress).toBe(
      "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B"
    );
    expect(result.document.proofs).toHaveLength(1);
    expect(result.document.proofs[0]!.type).toBe("ed25519");
  });

  it("payload_hash is keccak256(JCS(feedback))", async () => {
    const { jcsBytes } = await import("../../src/erc8004/jcs.js");
    const { keccak256Hex } = await import("../../src/erc8004/evm.js");
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    const expected = keccak256Hex(jcsBytes(result.document.feedback));
    expect(result.payloadHash).toBe(expected);
  });

  it("feedbackHash is keccak256(JCS(document))", async () => {
    const { jcsBytes } = await import("../../src/erc8004/jcs.js");
    const { keccak256Hex } = await import("../../src/erc8004/evm.js");
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    const expected = keccak256Hex(jcsBytes(result.document));
    expect(result.feedbackHash).toBe(expected);
  });

  it("stripping the proof changes feedbackHash (two-level design)", async () => {
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    const { jcsBytes } = await import("../../src/erc8004/jcs.js");
    const { keccak256Hex } = await import("../../src/erc8004/evm.js");
    const stripped = { ...result.document, proofs: [] };
    const strippedHash = keccak256Hex(jcsBytes(stripped));

    expect(strippedHash).not.toBe(result.feedbackHash);
  });

  it("onchain.selector matches GIVE_FEEDBACK_SELECTOR", async () => {
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    expect(result.onchain.selector).toBe("0x3c036a7e");
  });

  it("onchain.to is the mainnet Reputation Registry", async () => {
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    expect(result.onchain.to.toLowerCase()).toBe(
      "0x8004baa17c55a88189ae136b182e5fda19de9b63"
    );
  });

  it("preflight.requiredSender matches clientAddress", async () => {
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    expect(result.preflight.requiredSender).toBe(
      "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B"
    );
  });

  it("documentJson round-trips through JSON.parse", async () => {
    const signer = await makeSigner();
    const result = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    const parsed = JSON.parse(result.documentJson) as unknown;
    expect(parsed).toEqual(result.document);
  });

  it("is deterministic for same input and signer", async () => {
    const signer = await makeSigner();
    const r1 = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    const r2 = await prepareFeedback(fixtureInput(signer.pubkey), { signer });
    expect(r1.feedbackHash).toBe(r2.feedbackHash);
    expect(r1.documentJson).toBe(r2.documentJson);
  });

  it("rejects invalid clientAddress", async () => {
    const signer = await makeSigner();
    const input = { ...fixtureInput(signer.pubkey), clientAddress: "0xnotanaddress" };
    await expect(prepareFeedback(input, { signer })).rejects.toThrow(FeedbackError);
  });

  it("rejects note > 280 chars", async () => {
    const signer = await makeSigner();
    const input = { ...fixtureInput(signer.pubkey), note: "x".repeat(281) };
    await expect(prepareFeedback(input, { signer })).rejects.toThrow(FeedbackError);
  });

  it("accepts negative value (int128)", async () => {
    const signer = await makeSigner();
    const input = { ...fixtureInput(signer.pubkey), value: -100 };
    const result = await prepareFeedback(input, { signer });
    expect(result.document.feedback.value).toBe(-100);
  });
});

// ── verifyFeedbackDocument ─────────────────────────────────────────────────

describe("verifyFeedbackDocument", () => {
  it("verifies a document produced by prepareFeedback", async () => {
    const signer = await makeSigner();
    const prepared = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    const result = await verifyFeedbackDocument({
      documentJson: prepared.documentJson,
      onchainFeedbackHash: prepared.feedbackHash,
    });

    expect(result.valid).toBe(true);
    expect(result.checks.feedbackHash).toBe(true);
    expect(result.checks.payloadHash).toBe(true);
    expect(result.checks.ed25519).toBe(true);
    expect(result.checks.senderBinding).toBe("not-checked");
  });

  it("senderBinding verified when onchainSender matches clientAddress", async () => {
    const signer = await makeSigner();
    const prepared = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    const result = await verifyFeedbackDocument({
      documentJson: prepared.documentJson,
      onchainFeedbackHash: prepared.feedbackHash,
      onchainSender: "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",
    });

    expect(result.checks.senderBinding).toBe("verified");
    expect(result.valid).toBe(true);
  });

  it("senderBinding mismatch makes valid=false", async () => {
    const signer = await makeSigner();
    const prepared = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    const result = await verifyFeedbackDocument({
      documentJson: prepared.documentJson,
      onchainFeedbackHash: prepared.feedbackHash,
      onchainSender: "0x0000000000000000000000000000000000000001",
    });

    expect(result.checks.senderBinding).toBe("mismatch");
    expect(result.valid).toBe(false);
  });

  it("byte-tampering the document fails feedbackHash check", async () => {
    const signer = await makeSigner();
    const prepared = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    const tampered = prepared.documentJson.replace("9800", "9801");
    const result = await verifyFeedbackDocument({
      documentJson: tampered,
      onchainFeedbackHash: prepared.feedbackHash,
    });

    expect(result.checks.feedbackHash).toBe(false);
    expect(result.valid).toBe(false);
  });

  it("stripping the Ed25519 proof fails the hash check (two-level proof)", async () => {
    const signer = await makeSigner();
    const prepared = await prepareFeedback(fixtureInput(signer.pubkey), { signer });

    const doc = JSON.parse(prepared.documentJson) as Record<string, unknown>;
    (doc as { proofs: unknown[] }).proofs = [];
    const stripped = JSON.stringify(doc);

    const result = await verifyFeedbackDocument({
      documentJson: stripped,
      onchainFeedbackHash: prepared.feedbackHash,
    });

    // Stripping the proof changes the document → feedbackHash mismatch.
    expect(result.checks.feedbackHash).toBe(false);
    expect(result.valid).toBe(false);
  });

  it("returns error on invalid JSON", async () => {
    const result = await verifyFeedbackDocument({ documentJson: "{bad json" });
    expect(result.valid).toBe(false);
    expect(result.error).toMatch(/JSON.parse failed/);
  });

  it("returns error on wrong schema", async () => {
    const result = await verifyFeedbackDocument({
      documentJson: JSON.stringify({ schema: "WRONG", feedback: {}, payload_hash: "0x", proofs: [] }),
    });
    expect(result.valid).toBe(false);
  });
});
