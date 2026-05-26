#!/usr/bin/env node
// Generate legacy-identity.json deterministically from Ed25519 seed (32 bytes of 0x42).
// The secret field contains the 64-byte Solana keypair format (32-byte seed + 32-byte pubkey).
// Output: compact JSON (no whitespace) to stdout for reproducibility.

import { ed25519 } from "@noble/curves/ed25519";

// Ed25519 seed: 32 bytes of 0x42
const seed = new Uint8Array(32).fill(0x42);

// Derive public key from seed
const pubkey = ed25519.getPublicKey(seed);

// Format: secret = seed (32) + pubkey (32)
const secret = new Uint8Array([...seed, ...pubkey]);

// Convert to base58 for pubkey_base58 field
// Since we're using @noble/curves, we'll use a simple hex representation
// but actually, solana uses base58check. For testing, we'll use hex representation
// to make it deterministic and clear.
// Actually, we should use proper base58. Let's use a minimal base58 encoder.
function base58encode(bytes) {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let base58 = "";
  let num = 0n;
  for (const byte of bytes) {
    num = num * 256n + BigInt(byte);
  }
  if (num === 0n) return "1";
  while (num > 0n) {
    base58 = alphabet[Number(num % 58n)] + base58;
    num = num / 58n;
  }
  // Handle leading zeros
  for (const byte of bytes) {
    if (byte === 0) base58 = "1" + base58;
    else break;
  }
  return base58;
}

const pubkeyBase58 = base58encode(pubkey);

// Create legacy format object: {secret: [u8 array as numbers], pubkey_base58: string}
const legacy = {
  secret: Array.from(secret),
  pubkey_base58: pubkeyBase58,
};

// Emit compact JSON (no whitespace)
console.log(JSON.stringify(legacy));
