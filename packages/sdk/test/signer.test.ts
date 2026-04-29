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

// ── Defense-in-depth: WASM drift / corruption catch-paths ──────────────────
//
// These tests pin the three internal `LocalSigner.sign` guards that exist
// purely to surface SDK/WASM drift in production with a typed error rather
// than letting a silent bad signature reach the server. Each test installs
// a custom WASM mock with a single overridden `sign_challenge`.

describe("LocalSigner defense-in-depth", () => {
  function installSignChallenge(
    impl: (kp: unknown, bytes: Uint8Array) => Uint8Array
  ): void {
    // Build the standard mock then patch sign_challenge so KeypairJson
    // round-tripping (used by Keypair.generate) still works.
    const base = buildWasmMock();
    const patched = {
      ...base,
      sign_challenge: impl,
    };
    __setWasmForTesting(patched as never);
  }

  it("wraps thrown WASM errors as UserError", async () => {
    const kp = await Keypair.generate();
    installSignChallenge(() => {
      throw new Error("wasm panic: stack overflow");
    });
    const signer = new LocalSigner(kp);
    await expect(signer.sign(new Uint8Array([1, 2, 3]))).rejects.toBeInstanceOf(
      UserError
    );
    await expect(signer.sign(new Uint8Array([1, 2, 3]))).rejects.toThrow(
      /sign_challenge failed/
    );
  });

  it("rejects non-Uint8Array WASM return as UserError", async () => {
    const kp = await Keypair.generate();
    installSignChallenge(
      () =>
        // intentional bad return: a string masquerading as Uint8Array.
        "not-a-uint8array" as unknown as Uint8Array
    );
    const signer = new LocalSigner(kp);
    await expect(signer.sign(new Uint8Array([1, 2, 3]))).rejects.toBeInstanceOf(
      UserError
    );
    await expect(signer.sign(new Uint8Array([1, 2, 3]))).rejects.toThrow(
      /did not return Uint8Array/
    );
  });

  it("rejects wrong-length WASM signature as UserError", async () => {
    const kp = await Keypair.generate();
    installSignChallenge(() => new Uint8Array(63)); // off-by-one
    const signer = new LocalSigner(kp);
    await expect(signer.sign(new Uint8Array([1, 2, 3]))).rejects.toBeInstanceOf(
      UserError
    );
    await expect(signer.sign(new Uint8Array([1, 2, 3]))).rejects.toThrow(
      /must be 64 bytes/
    );
  });
});
