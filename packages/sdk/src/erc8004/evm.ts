// EVM utilities: keccak256, EIP-55 checksum address, hex encoding/decoding.
// No `node:` imports — runtime-agnostic (Node ≥20, Bun, Deno, Workers, browsers).

import { keccak_256 } from "@noble/hashes/sha3.js";

const encoder = new TextEncoder();

// ── keccak256 ──────────────────────────────────────────────────────────────

export function keccak256(bytes: Uint8Array): Uint8Array {
  return keccak_256(bytes);
}

// Returns lowercase hex with 0x prefix.
export function keccak256Hex(bytes: Uint8Array): `0x${string}` {
  return `0x${bytesToHex(keccak_256(bytes))}`;
}

// keccak256 of the UTF-8 encoding of a string.
export function keccak256Str(s: string): Uint8Array {
  return keccak_256(encoder.encode(s));
}

// ── hex encoding ───────────────────────────────────────────────────────────

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function hexToBytes(hex: string): Uint8Array {
  const h = hex.startsWith("0x") || hex.startsWith("0X") ? hex.slice(2) : hex;
  if (h.length % 2 !== 0) {
    throw new EvmError(`hex: odd-length string (${h.length} chars)`);
  }
  if (!/^[0-9a-fA-F]*$/.test(h)) {
    throw new EvmError("hex: invalid characters");
  }
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

// ── EIP-55 checksum address ────────────────────────────────────────────────

export function toChecksumAddress(address: string): `0x${string}` {
  const raw = address.toLowerCase().replace(/^0x/i, "");
  if (!/^[0-9a-f]{40}$/.test(raw)) {
    throw new EvmError(`invalid Ethereum address: ${address}`);
  }
  const hash = bytesToHex(keccak_256(encoder.encode(raw)));
  let result = "0x";
  for (let i = 0; i < 40; i++) {
    const nibble = parseInt(hash[i]!, 16);
    result += nibble >= 8 ? raw[i]!.toUpperCase() : raw[i]!;
  }
  return result as `0x${string}`;
}

// Returns true only if the address is 0x + 40 hex chars with correct EIP-55
// capitalisation. The zero address is valid per EIP-55 (all zeros hash →
// all lowercase, checksum passes).
export function isValidChecksumAddress(address: string): boolean {
  if (!/^0x[0-9a-fA-F]{40}$/.test(address)) return false;
  try {
    return toChecksumAddress(address) === address;
  } catch {
    return false;
  }
}

// ── uint256 big-endian ─────────────────────────────────────────────────────

// Encodes a non-negative safe integer as a 32-byte big-endian value.
export function encodeUint256(value: number | bigint): Uint8Array {
  const n = BigInt(value);
  if (n < 0n) throw new EvmError("uint256: negative value");
  const hex = n.toString(16).padStart(64, "0");
  if (hex.length > 64) throw new EvmError("uint256: value exceeds 256 bits");
  return hexToBytes(hex);
}

// Encodes a signed integer as a 32-byte two's-complement big-endian value.
// Accepts int128 range: -(2^127) to (2^127 - 1).
export function encodeInt128(value: number | bigint): Uint8Array {
  const n = BigInt(value);
  const MIN = -(2n ** 127n);
  const MAX = 2n ** 127n - 1n;
  if (n < MIN || n > MAX) {
    throw new EvmError(`int128: ${value} out of range [${MIN}, ${MAX}]`);
  }
  // Two's complement in 256 bits
  const twos = n < 0n ? n + 2n ** 256n : n;
  return hexToBytes(twos.toString(16).padStart(64, "0"));
}

// Encodes a uint8 value as 32 bytes (right-aligned, left-padded with zeros).
export function encodeUint8(value: number): Uint8Array {
  if (!Number.isInteger(value) || value < 0 || value > 255) {
    throw new EvmError(`uint8: ${value} out of range [0, 255]`);
  }
  const out = new Uint8Array(32);
  out[31] = value;
  return out;
}

// Encodes a bytes32 value (accepts 0x-prefixed hex or raw 32-byte Uint8Array).
export function encodeBytes32(value: string | Uint8Array): Uint8Array {
  if (typeof value === "string") {
    const bytes = hexToBytes(value);
    if (bytes.length !== 32) {
      throw new EvmError(`bytes32: expected 32 bytes, got ${bytes.length}`);
    }
    return bytes;
  }
  if (value.length !== 32) {
    throw new EvmError(`bytes32: expected 32 bytes, got ${value.length}`);
  }
  return value;
}

export class EvmError extends Error {
  override name = "EvmError";
}
