// Abstract contract suite for `Signer` implementations.
//
// Every concrete signer (LocalSigner today, future TurnkeySigner /
// WebAuthnSigner) MUST pass these assertions. Drives Decision 4 — the
// `Signer` interface guarantees behavioral equivalence across impls.
//
// Usage:
//   import { runSignerContract } from "./signer-contract.js";
//   describe("MySigner", () => runSignerContract(() => new MySigner(...)));
//
// Verification: we use `@noble/ed25519` (pure JS) for signature verification
// rather than relying on a WASM-side `verify_signature` export (which the
// WASM module does not expose today). Since Ed25519 is a public-key
// algorithm, any compliant verifier suffices — this is the same pattern
// the server uses (`identity::verify_signature`) just transplanted to JS.

import { describe, expect, it } from "vitest";
import * as ed from "@noble/ed25519";
import bs58 from "./helpers/bs58.js";
import type { Signer } from "../src/signer.js";

/**
 * Run the contract suite. Pass a factory so each test gets a fresh signer
 * (the suite never shares signer state across tests).
 */
export function runSignerContract(
  makeSigner: () => Signer | Promise<Signer>
): void {
  describe("Signer contract", () => {
    it("exposes a non-empty base58 pubkey", async () => {
      const signer = await makeSigner();
      expect(typeof signer.pubkey).toBe("string");
      expect(signer.pubkey.length).toBeGreaterThan(0);
      // base58 alphabet check — no 0/O/I/l, no '+'/'/'.
      expect(signer.pubkey).toMatch(/^[1-9A-HJ-NP-Za-km-z]+$/);
      // Decoded length is 32 bytes for Ed25519.
      const decoded = bs58.decode(signer.pubkey);
      expect(decoded.length).toBe(32);
    });

    it("returns a 64-byte signature for non-empty input", async () => {
      const signer = await makeSigner();
      const message = new Uint8Array(32);
      for (let i = 0; i < 32; i++) message[i] = i;
      const sig = await signer.sign(message);
      expect(sig).toBeInstanceOf(Uint8Array);
      expect(sig.length).toBe(64);
    });

    it("produces signatures that verify under the pubkey", async () => {
      const signer = await makeSigner();
      const message = new Uint8Array([
        0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04,
      ]);
      const sig = await signer.sign(message);
      const pubkeyBytes = bs58.decode(signer.pubkey);
      const ok = await ed.verifyAsync(sig, message, pubkeyBytes);
      expect(ok).toBe(true);
    });

    it("is deterministic — same input twice produces identical bytes", async () => {
      const signer = await makeSigner();
      const message = new TextEncoder().encode("deterministic-ed25519-test");
      const sig1 = await signer.sign(message);
      const sig2 = await signer.sign(message);
      expect(Buffer.from(sig1).equals(Buffer.from(sig2))).toBe(true);
    });

    it("rejects zero-length input", async () => {
      const signer = await makeSigner();
      await expect(signer.sign(new Uint8Array(0))).rejects.toThrow();
    });

    it("rejects null/undefined input", async () => {
      const signer = await makeSigner();
      // @ts-expect-error — intentional bad input
      await expect(signer.sign(null)).rejects.toThrow();
      // @ts-expect-error — intentional bad input
      await expect(signer.sign(undefined)).rejects.toThrow();
    });
  });
}
