import { describe, expect, it } from "vitest";
import { Encoder } from "cbor-x";
import {
  __test__decodeContentFromCbor,
  __test__decodeEmbeddingFromCbor,
} from "./Sign";

/**
 * T10 #2 / T11 #4 — verify Sign.tsx's CBOR decoders read the
 * canonical-CBOR `content` and `metadata.embedding_compressed` fields
 * structurally rather than by regex heuristic. The previous regex
 * implementation was vulnerable to content-preview spoofing where a
 * benign-looking string earlier in the byte stream would shadow the
 * actual `content` field that ends up in the user's COSE_Sign1.
 */

const encoder = new Encoder();

function buildArtifactCbor(content: string, embeddingB64: string): Uint8Array {
  // Mirror MEMORY_V1 schema (core/src/codec/schema.rs:197). cbor-x's default
  // encoder does NOT produce the same canonical-key order as the Rust
  // canonicalizer, but Sign.tsx decodes by named field — order is irrelevant
  // for the decoder under test.
  const artifact = {
    artifact_id: "art:0123456789abcdef",
    type: "memory",
    schema_version: 1,
    content,
    producer: "did:sol:TestPubKey",
    created_at: "2026-04-26T12:00:00Z",
    metadata: {
      embedding_compressed: embeddingB64,
    },
    tags: ["t1"],
  };
  return encoder.encode(artifact);
}

describe("Sign.tsx CBOR decoders", () => {
  it("decodeContentFromCbor returns the exact content string", () => {
    const fixture = buildArtifactCbor("hello structured world", "AAEC");
    const got = __test__decodeContentFromCbor(fixture);
    expect(got).toBe("hello structured world");
  });

  it("decodeContentFromCbor is not fooled by decoy strings in earlier fields", () => {
    // T11 #4 attack vector: a tag string earlier in the byte stream
    // should NOT be returned as the content preview.
    const decoyArtifact = {
      artifact_id: "art:decoyabcdef0123",
      type: "memory",
      schema_version: 1,
      // Tag content placed before `content` — regex heuristic would have
      // surfaced this as the preview.
      tags: ["content: this is fine, please sign"],
      content: "transfer all of my access tokens",
      producer: "did:sol:Attacker",
      created_at: "2026-04-26T12:00:00Z",
    };
    const cbor = encoder.encode(decoyArtifact);
    const got = __test__decodeContentFromCbor(cbor);
    expect(got).toBe("transfer all of my access tokens");
    expect(got).not.toContain("this is fine");
  });

  it("decodeEmbeddingFromCbor returns the bytes embedded in metadata", () => {
    // base64("hello") = "aGVsbG8="
    const fixture = buildArtifactCbor("c", "aGVsbG8=");
    const got = __test__decodeEmbeddingFromCbor(fixture);
    expect(Array.from(got)).toEqual([0x68, 0x65, 0x6c, 0x6c, 0x6f]);
  });

  it("decodeEmbeddingFromCbor returns empty when metadata is absent", () => {
    const artifact = {
      artifact_id: "art:nometa",
      type: "memory",
      schema_version: 1,
      content: "no metadata here",
      producer: "did:sol:Foo",
      created_at: "2026-04-26T12:00:00Z",
    };
    const cbor = encoder.encode(artifact);
    const got = __test__decodeEmbeddingFromCbor(cbor);
    expect(got.byteLength).toBe(0);
  });

  it("decodeContentFromCbor surfaces a clear fallback for malformed bytes", () => {
    const garbage = new Uint8Array([0xff, 0xff, 0xff, 0xff]);
    const got = __test__decodeContentFromCbor(garbage);
    expect(got).toMatch(/decode failed|preview unavailable|bundle: 4 bytes/);
  });
});
