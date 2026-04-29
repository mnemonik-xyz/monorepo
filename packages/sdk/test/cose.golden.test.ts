// Golden COSE round-trip — byte-for-byte parity test between Rust core and SDK.
//
// Reads `test/fixtures/golden-cose.json` (regenerated from
// `core/tests/golden_fixtures.rs::emit_fixtures`) and asserts the SDK's
// `coseSignPayload` produces byte-identical output for every entry. This is
// the user-facing implementation of tech-spec Decision 12.
//
// Why we import `core/pkg-nodejs/mnemonic_core.js` instead of using the
// SDK's `loadWasm()`:
//
//   - `loadWasm()` defaults to the `--target web` artifact, which calls
//     `fetch(file://...)` for `.wasm` loading. Vitest runs under Node, where
//     that path does not work without polyfills. Bun/Deno would, but the
//     SDK's vitest runner is Node-only.
//   - The `pkg-nodejs` build is a CommonJS-ish artifact that auto-loads
//     `.wasm` via `node:fs.readFileSync` and exposes the same surface. For
//     this test we install it via `__setWasmForTesting`, which is the single
//     test-mocking hook the SDK exposes.
//   - The byte-equality check is independent of which target was used: both
//     `--target web` and `--target nodejs` link the same Rust code and the
//     same Ed25519 + COSE crates, so their output is bit-identical.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { afterEach, beforeAll, describe, expect, it } from "vitest";

import { coseSignPayload } from "../src/cose.js";
import { __setWasmForTesting } from "../src/wasm.js";

interface GoldenEntry {
  name: string;
  input_hex: string;
  canonical_cbor_hex: string;
  cose_envelope_hex: string;
  keypair_secret_hex: string;
}

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const FIXTURE_PATH = resolve(__dirname, "fixtures/golden-cose.json");

function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) {
    throw new Error(`hex string length must be even, got ${hex.length}`);
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function bytesToHex(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i++) {
    s += bytes[i]!.toString(16).padStart(2, "0");
  }
  return s;
}

// Minimal base58 (Bitcoin alphabet) — only used to derive the
// `pubkey_base58` from the 32-byte pubkey suffix of `keypair_secret_hex`.
// We reuse the same alphabet and arithmetic the test helper uses.
const BS58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
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
  let out = "";
  for (let k = 0; k < zeros; k++) out += BS58_ALPHABET[0];
  for (let k = digits.length - 1; k >= 0; k--) {
    out += BS58_ALPHABET[digits[k]!];
  }
  return out;
}

let fixtures: GoldenEntry[] = [];
let realWasm: unknown = null;

beforeAll(async () => {
  // Load the committed fixture file. If this fails the test bails — the CI
  // lockstep gate would have caught an outdated checksum already.
  const raw = readFileSync(FIXTURE_PATH, "utf-8");
  fixtures = JSON.parse(raw) as GoldenEntry[];
  if (!Array.isArray(fixtures) || fixtures.length < 20) {
    throw new Error(
      `golden-cose.json must contain at least 20 entries, got ${fixtures.length}`
    );
  }
  // Import the real `--target nodejs` WASM artifact. Path is relative to
  // this test file: `packages/sdk/test/cose.golden.test.ts` ->
  // `core/pkg-nodejs/mnemonic_core.js` is four levels up.
  realWasm = await import("../../../core/pkg-nodejs/mnemonic_core.js");
});

afterEach(() => {
  __setWasmForTesting(null);
});

describe("golden COSE fixture (byte-for-byte parity with Rust core)", () => {
  it("fixture has at least 20 entries with unique names", () => {
    expect(fixtures.length).toBeGreaterThanOrEqual(20);
    const names = new Set(fixtures.map((f) => f.name));
    expect(names.size).toBe(fixtures.length);
  });

  it("all entries match WASM-produced COSE byte-for-byte", async () => {
    // Install the real pkg-nodejs WASM as the SDK's WASM source for this
    // test — `coseSignPayload` will call into it via `loadWasm()`.
    __setWasmForTesting(realWasm as never);

    const failures: Array<{ name: string; reason: string }> = [];

    for (const entry of fixtures) {
      const canonical = hexToBytes(entry.canonical_cbor_hex);
      const expectedCose = hexToBytes(entry.cose_envelope_hex);
      const secretBytes = hexToBytes(entry.keypair_secret_hex);
      if (secretBytes.length !== 64) {
        failures.push({
          name: entry.name,
          reason: `keypair_secret_hex must be 64 bytes, got ${secretBytes.length}`,
        });
        continue;
      }
      // Solana keypair = [seed(32) || pubkey(32)]. The KeypairJson shape
      // wants `pubkey_base58`, derived by base58-encoding the last 32 bytes.
      const pubkeyBytes = secretBytes.slice(32);
      const keypairJson = {
        secret: Array.from(secretBytes),
        pubkey_base58: bs58Encode(pubkeyBytes),
      };

      let actual: Uint8Array;
      try {
        actual = await coseSignPayload(canonical, keypairJson);
      } catch (e) {
        failures.push({
          name: entry.name,
          reason: `coseSignPayload threw: ${(e as Error).message}`,
        });
        continue;
      }
      if (actual.length !== expectedCose.length) {
        failures.push({
          name: entry.name,
          reason: `length mismatch: expected ${expectedCose.length}, got ${actual.length}`,
        });
        continue;
      }
      // Byte-for-byte comparison. Use hex on mismatch so the failure log is
      // trivially diff-able.
      const actualHex = bytesToHex(actual);
      if (actualHex !== entry.cose_envelope_hex) {
        failures.push({
          name: entry.name,
          reason: `byte mismatch:\n  expected: ${entry.cose_envelope_hex}\n  actual:   ${actualHex}`,
        });
      }
    }

    if (failures.length > 0) {
      const summary = failures
        .map((f) => `  - ${f.name}: ${f.reason}`)
        .join("\n");
      throw new Error(
        `${failures.length}/${fixtures.length} fixture(s) failed:\n${summary}`
      );
    }
  });
});
