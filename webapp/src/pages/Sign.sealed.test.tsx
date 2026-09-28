import { describe, expect, it } from "vitest";
import { Encoder } from "cbor-x";
import { __test__peekArtifactType, __test__decodeContentFromCbor } from "./Sign";

/**
 * Tests for the sealed bundle detection in Sign.tsx (tech-spec §8.1 step 5).
 *
 * Verifies:
 * - peekArtifactType correctly identifies SEALED_V1 vs MEMORY_V1 bundles.
 * - The fallback path for malformed CBOR is safe.
 */

const encoder = new Encoder();

function buildMemoryV1Cbor(content: string): Uint8Array {
  return encoder.encode({
    artifact_id: "art:mem-test-001",
    type: "memory",
    schema_version: 1,
    content,
    producer: "did:sol:TestPubkey",
    created_at: "2026-09-28T10:00:00Z",
    metadata: {},
  });
}

function buildSealedV1Cbor(): Uint8Array {
  return encoder.encode({
    artifact_id: "art:sealed-test-001",
    type: "sealed",
    schema_version: 1,
    alg: "xchacha20poly1305",
    nonce: new Uint8Array(24),
    ct: new Uint8Array(64),
    kc: new Uint8Array(32),
    wraps: [],
    created_at: "2026-09-28T10:00:00Z",
    producer: "did:sol:TestPubkey",
  });
}

describe("Sign.tsx — peekArtifactType", () => {
  it("returns 'memory' for a MEMORY_V1 bundle", () => {
    const cbor = buildMemoryV1Cbor("hello world");
    expect(__test__peekArtifactType(cbor)).toBe("memory");
  });

  it("returns 'sealed' for a SEALED_V1 bundle", () => {
    const cbor = buildSealedV1Cbor();
    expect(__test__peekArtifactType(cbor)).toBe("sealed");
  });

  it("returns 'unknown' for malformed CBOR", () => {
    const garbage = new Uint8Array([0xff, 0xff, 0xff]);
    expect(__test__peekArtifactType(garbage)).toBe("unknown");
  });

  it("returns 'unknown' when type field is missing", () => {
    const noType = encoder.encode({
      artifact_id: "art:no-type",
      schema_version: 1,
      content: "no type field",
    });
    expect(__test__peekArtifactType(noType)).toBe("unknown");
  });
});

describe("Sign.tsx — CBOR decoders handle SEALED_V1 safely", () => {
  it("decodeContentFromCbor returns a fallback for SEALED_V1 (no content field)", () => {
    const cbor = buildSealedV1Cbor();
    // A SEALED_V1 bundle has no `content` field — the decoder should return
    // the fallback message, not throw.
    const result = __test__decodeContentFromCbor(cbor);
    expect(result).toMatch(/bundle:.*content field missing/);
  });
});
