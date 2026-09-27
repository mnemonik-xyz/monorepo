// EVM utility tests: keccak256, EIP-55, hex, ABI primitives.

import { describe, expect, it } from "vitest";
import {
  bytesToHex,
  encodeBytes32,
  encodeInt128,
  encodeUint256,
  encodeUint8,
  EvmError,
  hexToBytes,
  isValidChecksumAddress,
  keccak256Hex,
  keccak256Str,
  toChecksumAddress,
} from "../../src/erc8004/evm.js";

describe("keccak256", () => {
  it("keccak256 of empty bytes = well-known hash", () => {
    const empty = new Uint8Array(0);
    expect(keccak256Hex(empty)).toBe(
      "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    );
  });

  it("keccak256Str of 'hello' = well-known hash", () => {
    expect(bytesToHex(keccak256Str("hello"))).toBe(
      "1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8"
    );
  });
});

describe("hexToBytes / bytesToHex", () => {
  it("roundtrips", () => {
    const hex = "deadbeef01020304";
    expect(bytesToHex(hexToBytes(hex))).toBe(hex);
  });

  it("accepts 0x prefix", () => {
    expect(bytesToHex(hexToBytes("0xdeadbeef"))).toBe("deadbeef");
  });

  it("rejects odd length", () => {
    expect(() => hexToBytes("abc")).toThrow(EvmError);
  });

  it("rejects invalid characters", () => {
    expect(() => hexToBytes("zz")).toThrow(EvmError);
  });
});

describe("toChecksumAddress / isValidChecksumAddress", () => {
  // Well-known EIP-55 test vectors from EIP itself.
  const vectors = [
    "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
    "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
    "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
    "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
  ];

  for (const addr of vectors) {
    it(`checksum: ${addr}`, () => {
      expect(toChecksumAddress(addr.toLowerCase())).toBe(addr);
      expect(isValidChecksumAddress(addr)).toBe(true);
    });
  }

  it("rejects wrong case", () => {
    expect(isValidChecksumAddress("0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed")).toBe(false);
  });

  it("rejects wrong length", () => {
    expect(isValidChecksumAddress("0x1234")).toBe(false);
  });

  it("rejects non-hex characters", () => {
    expect(isValidChecksumAddress("0xGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG")).toBe(false);
  });

  it("throws on invalid input", () => {
    expect(() => toChecksumAddress("notanaddress")).toThrow(EvmError);
  });
});

describe("encodeUint256", () => {
  it("zero", () => {
    const bytes = encodeUint256(0);
    expect(bytes.length).toBe(32);
    expect(bytes.every((b) => b === 0)).toBe(true);
  });

  it("encodes 1 as right-aligned big-endian", () => {
    const bytes = encodeUint256(1);
    expect(bytes[31]).toBe(1);
    expect(bytes.slice(0, 31).every((b) => b === 0)).toBe(true);
  });

  it("accepts bigint", () => {
    expect(encodeUint256(42n)[31]).toBe(42);
  });

  it("rejects negative", () => {
    expect(() => encodeUint256(-1)).toThrow(EvmError);
  });
});

describe("encodeInt128", () => {
  it("encodes 0", () => {
    const bytes = encodeInt128(0);
    expect(bytes.length).toBe(32);
    expect(bytes.every((b) => b === 0)).toBe(true);
  });

  it("encodes -1 as all 0xff", () => {
    const bytes = encodeInt128(-1n);
    expect(bytes.every((b) => b === 0xff)).toBe(true);
  });

  it("encodes positive value correctly", () => {
    const bytes = encodeInt128(9800);
    expect(bytes[31]).toBe(9800 & 0xff);
    // High bytes zero
    expect(bytes[0]).toBe(0);
  });

  it("rejects out-of-range", () => {
    expect(() => encodeInt128(2n ** 127n)).toThrow(EvmError);
    expect(() => encodeInt128(-(2n ** 127n) - 1n)).toThrow(EvmError);
  });
});

describe("encodeUint8", () => {
  it("encodes 0 and 255", () => {
    expect(encodeUint8(0)[31]).toBe(0);
    expect(encodeUint8(255)[31]).toBe(255);
    expect(encodeUint8(255).length).toBe(32);
  });

  it("rejects > 255", () => {
    expect(() => encodeUint8(256)).toThrow(EvmError);
  });

  it("rejects negative", () => {
    expect(() => encodeUint8(-1)).toThrow(EvmError);
  });
});

describe("encodeBytes32", () => {
  it("accepts 32-byte Uint8Array", () => {
    const bytes = new Uint8Array(32).fill(0xab);
    expect(encodeBytes32(bytes)).toEqual(bytes);
  });

  it("accepts 0x-prefixed hex", () => {
    const hex = "0x" + "ab".repeat(32);
    expect(encodeBytes32(hex)).toEqual(new Uint8Array(32).fill(0xab));
  });

  it("rejects wrong length", () => {
    expect(() => encodeBytes32(new Uint8Array(31))).toThrow(EvmError);
    expect(() => encodeBytes32("0x" + "ab".repeat(31))).toThrow(EvmError);
  });
});
