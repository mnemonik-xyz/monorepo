// Unit tests for the `Keypair` helper.
//
// We use the WASM mock from helpers/wasm-mock so we don't pay the real WASM
// init cost — the mock implements `generate_keypair`, `import_keypair_json`,
// `export_keypair_json` with the same contracts (validates secret length,
// round-trips via JSON).

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { IdentityRequiresKeystore, UserError } from "../src/errors.js";
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
    const original = a.secret[0];
    // Pick a sentinel guaranteed-different from the original byte so the
    // assertion doesn't false-fail ~1/256 runs when RNG happens to produce
    // secret[0] === sentinel.
    const sentinel = (original ^ 0xff) & 0xff;
    a.secret[0] = sentinel;
    const b = kp.toJSON();
    expect(b.secret[0]).toBe(original);
    expect(b.secret[0]).not.toBe(sentinel);
  });

  it("fromJSON round-trips a generated keypair", async () => {
    const kp1 = await Keypair.generate();
    const kp2 = await Keypair.fromJSON(kp1.toJSON());
    expect(kp2.pubkey).toBe(kp1.pubkey);
  });

  it("fromJSON accepts legacy shape and round-trips pubkey", async () => {
    const kp1 = await Keypair.generate();
    const legacy = kp1.toJSON();
    const kp2 = await Keypair.fromJSON(
      JSON.parse(JSON.stringify(legacy)) as unknown,
    );
    expect(kp2.pubkey).toBe(kp1.pubkey);
  });

  it("fromJSON with wrong secret length throws TypeError mentioning length", async () => {
    const err = await Keypair.fromJSON({
      secret: [1, 2, 3],
      pubkey_base58: "x",
    }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(UserError);
    expect((err as Error).message).toMatch(/3/);
  });

  it("fromJSON with stub shape throws IdentityRequiresKeystore", async () => {
    const stubPubkey = "So1anaFakePubkey111111111111111111111111111";
    const stubRef = "xyz.mnemonik.identity/default";
    const err = await Keypair.fromJSON({
      pubkey_base58: stubPubkey,
      keychain_ref: stubRef,
    }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(IdentityRequiresKeystore);
    const irk = err as IdentityRequiresKeystore;
    expect(irk.pubkey_base58).toBe(stubPubkey);
    expect(irk.keychain_ref).toBe(stubRef);
    expect(irk.message).toContain(stubRef);
    expect(irk.name).toBe("IdentityRequiresKeystore");
  });

  it("fromJSON with garbage object throws TypeError", async () => {
    await expect(
      // @ts-expect-error — intentional bad input
      Keypair.fromJSON({ wrong: "shape" }),
    ).rejects.toBeInstanceOf(TypeError);
  });

  it("fromJSON with non-object throws TypeError — null", async () => {
    await expect(Keypair.fromJSON(null)).rejects.toBeInstanceOf(TypeError);
  });

  it("fromJSON with non-object throws TypeError — number", async () => {
    await expect(Keypair.fromJSON(42)).rejects.toBeInstanceOf(TypeError);
  });

  it("fromJSON with non-object throws TypeError — string", async () => {
    await expect(Keypair.fromJSON("not-an-object")).rejects.toBeInstanceOf(
      TypeError,
    );
  });

  it("fromJSON rejects garbage shapes (legacy test — preserved)", async () => {
    await expect(
      // @ts-expect-error — intentional bad input
      Keypair.fromJSON({ wrong: "shape" }),
    ).rejects.toBeInstanceOf(TypeError);
  });

  it("fromJSON rejects wrong-length secret (legacy test — preserved)", async () => {
    await expect(
      Keypair.fromJSON({ secret: [1, 2, 3], pubkey_base58: "abc" }),
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
      UserError,
    );
  });
});
