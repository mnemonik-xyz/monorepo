// ABI encoder tests for ReputationRegistry.giveFeedback.
//
// Fixture generated with:
//   node --input-type=module < scripts/gen-erc8004-fixture.mjs
// (see bottom of this file for the equivalent manual check)

import { describe, expect, it } from "vitest";
import { encodeGiveFeedback } from "../../src/erc8004/abi.js";
import { bytesToHex, keccak256Str } from "../../src/erc8004/evm.js";
import {
  GIVE_FEEDBACK_SELECTOR,
  GIVE_FEEDBACK_SIG,
} from "../../src/erc8004/registry.js";

// ── Deterministic fixture args ────────────────────────────────────────────
const FIXTURE_ARGS = {
  agentId: 42n,
  value: 9800n,
  valueDecimals: 2,
  tag1: "quality",
  tag2: "research",
  endpoint: "https://agent.example.com/a2a",
  feedbackURI: "https://example.com/feedback/1.json",
  // exactly 32 bytes
  feedbackHash: "0x" + "ab".repeat(32),
} as const;

// ── Selector ──────────────────────────────────────────────────────────────

describe("giveFeedback selector", () => {
  it("pinned selector matches keccak256(signature)[0:4]", () => {
    const hash = keccak256Str(GIVE_FEEDBACK_SIG);
    const computed = "0x" + bytesToHex(hash.slice(0, 4));
    expect(computed).toBe(GIVE_FEEDBACK_SELECTOR);
    expect(GIVE_FEEDBACK_SELECTOR).toBe("0x3c036a7e");
  });
});

// ── Calldata structure ────────────────────────────────────────────────────

describe("encodeGiveFeedback", () => {
  it("calldata starts with the correct 4-byte selector", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    const selector = "0x" + bytesToHex(calldata.slice(0, 4));
    expect(selector).toBe(GIVE_FEEDBACK_SELECTOR);
  });

  it("calldata is 548 bytes for fixture args", () => {
    // 4 (selector) + 256 (head: 8×32) + 64 (tag1: 7 bytes, padded) +
    // 64 (tag2: 8 bytes, padded) + 64 (endpoint: 29 bytes, padded) +
    // 96 (feedbackURI: 35 bytes, 2×32 padded) = 548
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    expect(calldata.length).toBe(548);
  });

  it("agentId is encoded at head slot 0 (bytes 4–35)", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    // agentId = 42 = 0x2a, right-aligned in 32 bytes
    expect(calldata[35]).toBe(0x2a);
    expect(calldata.slice(4, 35).every((b) => b === 0)).toBe(true);
  });

  it("value (int128 9800 = 0x2648) is at head slot 1 (bytes 36–67)", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    expect(calldata[66]).toBe(0x26);
    expect(calldata[67]).toBe(0x48);
    expect(calldata.slice(36, 66).every((b) => b === 0)).toBe(true);
  });

  it("valueDecimals (2) is at head slot 2 (bytes 68–99)", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    expect(calldata[99]).toBe(2);
    expect(calldata.slice(68, 99).every((b) => b === 0)).toBe(true);
  });

  it("tag1 offset slot points to byte 256 (0x100)", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    // slot 3 = bytes 100–131: offset = 256 (0x0100)
    expect(calldata[130]).toBe(0x01);
    expect(calldata[131]).toBe(0x00);
    expect(calldata.slice(100, 130).every((b) => b === 0)).toBe(true);
  });

  it("feedbackHash (bytes32) is at head slot 7 (bytes 228–259)", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    // feedbackHash = 0xabab...ab (32 bytes)
    expect(calldata.slice(228, 260).every((b) => b === 0xab)).toBe(true);
  });

  it("tag1 'quality' appears in tail at byte 256", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    // tail starts at byte 4 + 256 = 260; offset 256 from body start = byte 4 + 256 = 260
    const tailStart = 4 + 256; // = 260
    // length prefix = 7 (0x07), data = "quality"
    expect(calldata[tailStart + 31]).toBe(7);
    const text = new TextDecoder().decode(calldata.slice(tailStart + 32, tailStart + 32 + 7));
    expect(text).toBe("quality");
  });

  it("negative value is two's-complement encoded", () => {
    const calldata = encodeGiveFeedback({ ...FIXTURE_ARGS, value: -1n });
    // -1 in two's complement (256 bits) = all 0xff bytes
    expect(calldata.slice(36, 68).every((b) => b === 0xff)).toBe(true);
  });

  it("encodes empty strings without throwing", () => {
    expect(() =>
      encodeGiveFeedback({ ...FIXTURE_ARGS, tag1: "", tag2: "", endpoint: "" })
    ).not.toThrow();
  });

  it("encodes unicode strings correctly", () => {
    const calldata = encodeGiveFeedback({ ...FIXTURE_ARGS, tag1: "🔑" });
    // "🔑" is 4 UTF-8 bytes (U+1F511)
    const tailStart = 4 + 256;
    expect(calldata[tailStart + 31]).toBe(4); // length = 4
  });

  // ── Pinned calldata fixture ─────────────────────────────────────────────
  //
  // To reproduce independently (no Mnemonic code):
  //   cast calldata "giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)" \
  //     42 9800 2 "quality" "research" "https://agent.example.com/a2a" \
  //     "https://example.com/feedback/1.json" \
  //     0xabababababababababababababababababababababababababababababababababab
  //
  // Expected output must equal PINNED_CALLDATA below.

  // Generated by running encodeGiveFeedback(FIXTURE_ARGS) and capturing output.
  // To reproduce independently (requires cast / foundry):
  //   cast calldata \
  //     "giveFeedback(uint256,int128,uint8,string,string,string,string,bytes32)" \
  //     42 9800 2 "quality" "research" "https://agent.example.com/a2a" \
  //     "https://example.com/feedback/1.json" \
  //     0xabababababababababababababababababababababababababababababababababab
  // prettier-ignore
  const PINNED_CALLDATA = "0x3c036a7e000000000000000000000000000000000000000000000000000000000000002a0000000000000000000000000000000000000000000000000000000000002648000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000140000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000001c0abababababababababababababababababababababababababababababababab00000000000000000000000000000000000000000000000000000000000000077175616c6974790000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000087265736561726368000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001d68747470733a2f2f6167656e742e6578616d706c652e636f6d2f613261000000000000000000000000000000000000000000000000000000000000000000002368747470733a2f2f6578616d706c652e636f6d2f666565646261636b2f312e6a736f6e0000000000000000000000000000000000000000000000000000000000";

  it("matches pinned calldata fixture", () => {
    const calldata = encodeGiveFeedback(FIXTURE_ARGS);
    const hex = "0x" + bytesToHex(calldata);
    expect(hex).toBe(PINNED_CALLDATA.replace(/\s/g, "").replace(/\n/g, ""));
  });
});
