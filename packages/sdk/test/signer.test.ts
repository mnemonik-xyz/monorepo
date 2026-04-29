// Unit tests for `LocalSigner` + the abstract `Signer` contract suite.
//
// Drives Decision 4 (Signer interface): `LocalSigner` runs through
// `runSignerContract(...)` cleanly. Every future signer impl must too.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { Keypair } from "../src/keypair.js";
import { LocalSigner } from "../src/signer.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { UserError } from "../src/errors.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";
import { runSignerContract } from "./signer-contract.js";

beforeEach(() => {
  // Each test gets a fresh mock so `__calls` doesn't leak between tests.
  __setWasmForTesting(buildWasmMock() as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

// ── TDD anchor: local_signer_runs_contract ─────────────────────────────────
describe("LocalSigner", () => {
  // The contract suite is the load-bearing assertion — it covers pubkey
  // shape, sig length, deterministic Ed25519, verify round-trip, and
  // empty/null rejection. See test/signer-contract.ts.
  runSignerContract(async () => {
    const kp = await Keypair.generate();
    return new LocalSigner(kp);
  });

  it("exposes the same pubkey as the underlying Keypair", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    expect(signer.pubkey).toBe(kp.pubkey);
  });

  it("rejects non-Uint8Array input with UserError", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    // @ts-expect-error — intentional bad input
    await expect(signer.sign("not bytes")).rejects.toBeInstanceOf(UserError);
  });

  it("rejects zero-length input with UserError", async () => {
    const kp = await Keypair.generate();
    const signer = new LocalSigner(kp);
    await expect(signer.sign(new Uint8Array(0))).rejects.toBeInstanceOf(
      UserError
    );
  });
});
