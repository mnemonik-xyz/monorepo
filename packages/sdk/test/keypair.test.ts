// Unit tests for the `Keypair` helper.
//
// We use the WASM mock from helpers/wasm-mock so we don't pay the real WASM
// init cost — the mock implements `generate_keypair`, `import_keypair_json`,
// `export_keypair_json` with the same contracts (validates secret length,
// round-trips via JSON).

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { UserError } from "../src/errors.js";
import { Keypair } from "../src/keypair.js";
import { __setWasmForTesting } from "../src/wasm.js";
import { buildWasmMock } from "./helpers/wasm-mock.js";

beforeEach(() => {
  __setWasmForTesting(buildWasmMock() as never);
});

afterEach(() => {
  __setWasmForTesting(null);
});

describe("Keypair", () => {
  it("generate produces a 64-byte secret + base58 pubkey", async () => {
    const kp = await Keypair.generate();
    const json = kp.toJSON();
    expect(json.secret.length).toBe(64);
    expect(typeof json.pubkey_base58).toBe("string");
    expect(json.pubkey_base58.length).toBeGreaterThan(0);
    expect(kp.pubkey).toBe(json.pubkey_base58);
  });

  it("toJSON returns a defensive copy", async () => {
    const kp = await Keypair.generate();
    const a = kp.toJSON();
    a.secret[0] = 0xff;
    const b = kp.toJSON();
    expect(b.secret[0]).not.toBe(0xff);
  });

  it("fromJSON round-trips a generated keypair", async () => {
    const kp1 = await Keypair.generate();
    const kp2 = await Keypair.fromJSON(kp1.toJSON());
    expect(kp2.pubkey).toBe(kp1.pubkey);
  });

  it("fromJSON rejects garbage shapes", async () => {
    await expect(
      // @ts-expect-error — intentional bad input
      Keypair.fromJSON({ wrong: "shape" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("fromJSON rejects wrong-length secret", async () => {
    await expect(
      Keypair.fromJSON({ secret: [1, 2, 3], pubkey_base58: "abc" })
    ).rejects.toBeInstanceOf(UserError);
  });

  it("toBackupString + fromBackupString round-trip", async () => {
    const kp = await Keypair.generate();
    const s = await kp.toBackupString();
    expect(typeof s).toBe("string");
    const kp2 = await Keypair.fromBackupString(s);
    expect(kp2.pubkey).toBe(kp.pubkey);
  });

  it("fromBackupString rejects malformed input", async () => {
    await expect(Keypair.fromBackupString("{not json")).rejects.toBeInstanceOf(
      UserError
    );
  });
});
