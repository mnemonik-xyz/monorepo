/**
 * Wave 2 of work/arweave-as-source-of-truth: a client must be able to rebuild a
 * recall row from the signed bytes stored on Arweave, with no server and no
 * native binary in the path.
 *
 * `core/src/rebuild.rs` used to be gated to native targets, so only the
 * operator could reconstruct an index. It is portable now, and `rebuild_row` is
 * exported from the WASM core. This test exercises the whole browser path for
 * real: build a full MEMORY_V1 artifact with the WASM core, sign it, then
 * rebuild the row from its COSE_Sign1 bytes and check every recovered field.
 *
 * Loads the `--target nodejs` artifact directly, for the reasons documented at
 * the top of `cose.golden.test.ts` (the `web` artifact `fetch`es its `.wasm`,
 * which does not work under Node).
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { beforeAll, describe, expect, it } from "vitest";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
let wasm: any;
let keypair: { secret: number[]; pubkey_base58: string };

// Base58 (Bitcoin alphabet), to derive `pubkey_base58` from the 32-byte pubkey
// suffix of the secret — same approach as `cose.golden.test.ts`.
const BS58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
function bs58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  const digits: number[] = [];
  for (let i = zeros; i < bytes.length; i++) {
    let carry = bytes[i]!;
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j]! * 256;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  let out = "1".repeat(zeros);
  for (let i = digits.length - 1; i >= 0; i--) out += BS58[digits[i]!];
  return out;
}

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return out;
}

beforeAll(async () => {
  wasm = await import("../../../core/pkg-nodejs/mnemonic_core.js");
  // A fixed keypair, not `generate_keypair()`: under vitest the nodejs WASM
  // artifact cannot reach Node's randomness backend, and a deterministic key
  // makes the test reproducible anyway. Reuses the committed golden fixture's
  // secret so there is no second source of test key material.
  const dir = dirname(fileURLToPath(import.meta.url));
  const fixtures = JSON.parse(
    readFileSync(resolve(dir, "fixtures/golden-cose.json"), "utf-8")
  ) as Array<{ keypair_secret_hex: string }>;
  const secret = hexToBytes(fixtures[0]!.keypair_secret_hex);
  keypair = {
    secret: Array.from(secret),
    pubkey_base58: bs58Encode(secret.slice(32)),
  };
});

const CONTENT = "rebuilt from arweave bytes, client-side";
const DIM = 8;

function buildSignedArtifact() {
  const embedding = Float32Array.from([0.5, -0.25, 0.125, 1, -1, 0, 0.75, -0.5]);
  const compressed = wasm.compress_embedding(embedding, 4);
  // Builds the MEMORY_V1 artifact (including metadata.embedding_compressed)
  // and returns COSE_Sign1 bytes — the same bytes an anchored write uploads.
  const cose = wasm.sign_attestation_bundle(
    CONTENT,
    compressed,
    "",
    keypair.pubkey_base58,
    keypair
  );
  return { embedding, compressed, cose };
}

describe("rebuild_row (client-side restore from Arweave bytes)", () => {
  it("reconstructs every signed field from the COSE bytes alone", () => {
    const { cose } = buildSignedArtifact();
    const row = wasm.rebuild_row(cose);

    expect(row.content).toBe(CONTENT);
    expect(row.owner_pubkey).toBe(keypair.pubkey_base58);
    expect(row.signer_pubkey).toBe(keypair.pubkey_base58);
    expect(row.content_hash).toMatch(/^[0-9a-f]{64}$/);
    expect(row.attestation_id.length).toBeGreaterThan(0);
  });

  it("recovers the embedding the producer's compressor would yield", () => {
    const { compressed, cose } = buildSignedArtifact();
    const row = wasm.rebuild_row(cose);
    const expected = wasm.decompress_embedding(compressed, DIM);

    expect(row.embedding.length).toBe(DIM);
    expect(Array.from(row.embedding)).toEqual(Array.from(expected));
  });

  it("reports the precision tier so a client can surface search quality", () => {
    const { cose } = buildSignedArtifact();
    // This artifact carries only the compressed copy, so recall over a restored
    // index is approximate. A public anchored write adds `embedding_f32` and
    // this becomes "f32".
    expect(wasm.rebuild_row(cose).precision).toBe("compressed");
  });

  it("refuses a tampered artifact instead of returning a row", () => {
    const { cose } = buildSignedArtifact();
    const tampered = Uint8Array.from(cose);
    tampered[tampered.length - 1]! ^= 0xff;
    expect(() => wasm.rebuild_row(tampered)).toThrow();
  });

  it("refuses an artifact that carries no embedding at all", () => {
    // The committed golden fixtures are minimal MEMORY_V1 artifacts built for
    // canonicalization parity; they carry no `metadata.embedding_compressed`.
    // Their signatures verify, but there is nothing to rebuild an index from, so
    // this must be an error rather than a row with an empty vector.
    const dir = dirname(fileURLToPath(import.meta.url));
    const fixtures = JSON.parse(
      readFileSync(resolve(dir, "fixtures/golden-cose.json"), "utf-8")
    ) as Array<{ cose_envelope_hex: string }>;
    const cose = hexToBytes(fixtures[0]!.cose_envelope_hex);
    expect(() => wasm.rebuild_row(cose)).toThrow(/embedding/i);
  });
});
