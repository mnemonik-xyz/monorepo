// Minimal base58 encode/decode (Bitcoin alphabet) — test-only helper.
//
// We don't pull in a third-party `bs58` dep just for tests; this is ~30
// lines of arithmetic. Correctness is verified by round-tripping random
// 32-byte inputs against the WASM-produced pubkey strings.

const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const ALPHABET_MAP = new Map<string, number>();
for (let i = 0; i < ALPHABET.length; i++) ALPHABET_MAP.set(ALPHABET[i]!, i);

function encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  // Count leading zeros.
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;

  // Big-endian byte array → base-58 digits via repeated division.
  const digits: number[] = [];
  for (let i = zeros; i < bytes.length; i++) {
    let carry = bytes[i]!;
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j]! * 256;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }

  let s = "";
  for (let i = 0; i < zeros; i++) s += ALPHABET[0];
  for (let i = digits.length - 1; i >= 0; i--) s += ALPHABET[digits[i]!];
  return s;
}

function decode(input: string): Uint8Array {
  if (input.length === 0) return new Uint8Array(0);
  let zeros = 0;
  while (zeros < input.length && input[zeros] === ALPHABET[0]) zeros++;

  const bytes: number[] = [];
  for (let i = zeros; i < input.length; i++) {
    const digit = ALPHABET_MAP.get(input[i]!);
    if (digit === undefined) {
      throw new Error(`bs58: invalid character '${input[i]}' at index ${i}`);
    }
    let carry = digit;
    for (let j = 0; j < bytes.length; j++) {
      carry += bytes[j]! * 58;
      bytes[j] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }

  const out = new Uint8Array(zeros + bytes.length);
  for (let i = 0; i < zeros; i++) out[i] = 0;
  for (let i = 0; i < bytes.length; i++)
    out[zeros + i] = bytes[bytes.length - 1 - i]!;
  return out;
}

export default { encode, decode };
