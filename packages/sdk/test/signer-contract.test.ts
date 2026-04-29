// Standalone runner for the abstract Signer contract suite against
// `LocalSigner`. Mirrors what `signer.test.ts` does inline — kept as a
// separate file so future signer impls (TurnkeySigner, WebAuthnSigner) can
// be added with one line each:
//
//   describe("TurnkeySigner", () => runSignerContract(makeTurnkeySigner));
//
// without disturbing the LocalSigner-specific tests.

import { afterEach, beforeEach } from "vitest";

import { Keypair } from "../src/keypair.js";
import { LocalSigner } from "../src/signer.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";
import { runSignerContract } from "./signer-contract.js";

beforeEach(() => {
  __setWasmForTesting(buildWasmMock() as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

runSignerContract(async () => {
  const kp = await Keypair.generate();
  return new LocalSigner(kp);
});
