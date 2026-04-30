// Unit tests for `coseSignPayload` — input validation + envelope shape check.
//
// The byte-level WASM correctness is covered by Task 4's golden fixture
// (against the real `--target web` artifact). Here we only verify that the
// SDK boundary rejects bad inputs and surfaces drift loudly.

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { coseSignPayload } from "../src/cose.js";
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

describe("coseSignPayload", () => {
  it("returns a non-empty COSE envelope starting with 0x84", async () => {
    const kp = await Keypair.generate();
    const payload = new Uint8Array([0xa1, 0x64, 0x68, 0x65, 0x6c, 0x6c, 0x6f]);
    const cose = await coseSignPayload(payload, kp.toJSON());
    expect(cose).toBeInstanceOf(Uint8Array);
    expect(cose.length).toBeGreaterThan(0);
    expect(cose[0]).toBe(0x84);
  });

  it("rejects non-Uint8Array payload with UserError", async () => {
    const kp = await Keypair.generate();
    await expect(
      // @ts-expect-error — intentional bad input
      coseSignPayload("not bytes", kp.toJSON())
    ).rejects.toBeInstanceOf(UserError);
  });

  it("rejects empty payload with UserError", async () => {
    const kp = await Keypair.generate();
    await expect(
      coseSignPayload(new Uint8Array(0), kp.toJSON())
    ).rejects.toBeInstanceOf(UserError);
  });

  it("wraps WASM throw under UserError", async () => {
    // Override sign_cose_payload to throw — verifies the catch path.
    const mock = buildWasmMock();
    const broken = {
      ...mock,
      sign_cose_payload: () => {
        throw new Error("synthetic wasm failure");
      },
    };
    __setWasmForTesting(broken as never);
    const kp = await Keypair.generate();
    await expect(
      coseSignPayload(new Uint8Array([1, 2, 3]), kp.toJSON())
    ).rejects.toBeInstanceOf(UserError);
  });

  it("rejects non-Uint8Array WASM return value", async () => {
    const mock = buildWasmMock();
    const broken = {
      ...mock,
      sign_cose_payload: () => "definitely not bytes" as unknown as Uint8Array,
    };
    __setWasmForTesting(broken as never);
    const kp = await Keypair.generate();
    await expect(
      coseSignPayload(new Uint8Array([1, 2, 3]), kp.toJSON())
    ).rejects.toThrow(/Uint8Array/);
  });

  it("rejects envelope without 0x84 prefix", async () => {
    const mock = buildWasmMock();
    const broken = {
      ...mock,
      sign_cose_payload: () => new Uint8Array([0x00, 0x01, 0x02]),
    };
    __setWasmForTesting(broken as never);
    const kp = await Keypair.generate();
    await expect(
      coseSignPayload(new Uint8Array([1, 2, 3]), kp.toJSON())
    ).rejects.toThrow(/0x84/);
  });
});
