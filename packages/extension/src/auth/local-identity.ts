// Local Ed25519 identity bootstrap helper (T25b).
//
// On every popup open we want a usable identity — the user should never
// have to paste a console snippet to mint one. `ensureLocalIdentity`
// looks under `chrome.storage.local` for the canonical identity keys
// (`identity` + `identity_secret`), validates them, and otherwise
// generates a fresh Ed25519 keypair using WebCrypto. The generated key
// is persisted under the same canonical keys so the existing runtime
// code (`runtime-impl.ts::loadIdentity`, COSE sign path) picks it up
// without further changes.
//
// Solana keypair shape: a 64-byte buffer composed of `seed (32B) ||
// public key (32B)`. WebCrypto's `crypto.subtle.generateKey('Ed25519',
// true, ...)` returns a CryptoKeyPair; the private key, when exported
// as PKCS#8, wraps the 32-byte seed at offset 16..48. The public key
// exports as 32 raw bytes via SPKI offset 12..44 (RFC 8410 §4 — SPKI
// header is fixed for Ed25519). We use those slices verbatim.
//
// Base58 encoding is inline (~30 lines) — we intentionally avoid a
// new `bs58` npm dep per the task spec.

/** Canonical storage keys — mirror `runtime-impl.ts::loadIdentity` and
 *  the rest of the popup. Anything else is a separate concept and must
 *  not collide. */
const IDENTITY_KEY = "identity";
const IDENTITY_SECRET_KEY = "identity_secret";

/** Bitcoin alphabet — same ordering MultiBase / Solana use. Reuse here
 *  keeps base58 strings copy-compatible with existing keypairs. */
const BS58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Output shape — identical to `runtime-impl.ts::FullIdentity` so the
 *  existing sign-path consumer can pass through without coercion. */
export interface LocalIdentity {
  pubkey_base58: string;
  /** 64-byte Solana keypair as a plain number[] (so `chrome.storage`
   *  can round-trip without binary coercion). */
  secret: number[];
}

/** Encode `bytes` as base58 (Bitcoin alphabet). Allocation-light; small
 *  enough that the popup pays the cost on first-paint without a worry. */
export function base58Encode(bytes: Uint8Array): string {
  if (bytes.length === 0) return "";
  // Count leading zero bytes — base58 encodes each as a literal "1".
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  // Convert big-endian bytes → base58 digits via repeated /58 mod58.
  const input = Array.from(bytes);
  const encoded: number[] = [];
  let start = zeros;
  while (start < input.length) {
    let remainder = 0;
    for (let i = start; i < input.length; i++) {
      const acc = (input[i] ?? 0) + remainder * 256;
      input[i] = Math.floor(acc / 58);
      remainder = acc % 58;
    }
    encoded.push(remainder);
    while (start < input.length && input[start] === 0) start++;
  }
  let out = "";
  for (let i = 0; i < zeros; i++) out += BS58_ALPHABET[0];
  for (let i = encoded.length - 1; i >= 0; i--) {
    const idx = encoded[i] ?? 0;
    out += BS58_ALPHABET[idx];
  }
  return out;
}

/** Shape we look up in `chrome.storage.local`. Stored values may be
 *  partial/legacy — `ensureLocalIdentity` validates before trusting. */
interface StoredIdentityShape {
  identity?: { pubkey_base58?: string } | null;
  identity_secret?: number[] | null;
}

function isValidStoredIdentity(s: StoredIdentityShape): s is {
  identity: { pubkey_base58: string };
  identity_secret: number[];
} {
  const pub = s.identity?.pubkey_base58;
  const sec = s.identity_secret;
  return (
    typeof pub === "string" &&
    pub.length > 0 &&
    Array.isArray(sec) &&
    sec.length === 64
  );
}

/**
 * Read the persisted identity if present, else generate + persist a
 * fresh Ed25519 keypair. Idempotent on subsequent popup mounts —
 * existing-and-valid identities are returned as-is so we never re-mint
 * a key behind the user's back.
 *
 * Storage failures (denied permissions, quota) bubble up — callers
 * (popup boot) wrap in try/catch and fall back to the legacy "no
 * identity" header so the popup still renders.
 */
export async function ensureLocalIdentity(): Promise<LocalIdentity> {
  const stored = (await chrome.storage.local.get([
    IDENTITY_KEY,
    IDENTITY_SECRET_KEY,
  ])) as StoredIdentityShape;
  if (isValidStoredIdentity(stored)) {
    return {
      pubkey_base58: stored.identity.pubkey_base58,
      secret: stored.identity_secret,
    };
  }
  const fresh = await generateEd25519Keypair();
  await chrome.storage.local.set({
    [IDENTITY_KEY]: { pubkey_base58: fresh.pubkey_base58 },
    [IDENTITY_SECRET_KEY]: fresh.secret,
  });
  return fresh;
}

/**
 * Generate a fresh Ed25519 keypair via WebCrypto and shape it as a
 * 64-byte Solana keypair (seed || pubkey). Throws if WebCrypto is
 * unavailable or rejects Ed25519 — the caller surfaces this as the
 * legacy "no identity" path.
 *
 * NOTE: the raw secret never leaves this module by reference — we copy
 * out the 32-byte seed + 32-byte pubkey into a fresh `Uint8Array` and
 * discard the CryptoKey. Callers receive the `number[]` form so a
 * `console.log` on the popup never accidentally surfaces a typed-array
 * view of live key material.
 */
async function generateEd25519Keypair(): Promise<LocalIdentity> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle || typeof subtle.generateKey !== "function") {
    throw new Error("ensureLocalIdentity: WebCrypto subtle unavailable");
  }
  // `Ed25519` lands as a first-class algorithm name in modern Chrome
  // (M114+); the Manifest V3 minimum is also M114, so we can rely on
  // it without a polyfill. `extractable: true` is required so we can
  // export the seed bytes below.
  const kp = (await subtle.generateKey(
    { name: "Ed25519" } as unknown as AlgorithmIdentifier,
    true,
    ["sign", "verify"],
  )) as CryptoKeyPair;
  // PKCS#8 export — the 32-byte seed sits at offset 16..48 of the
  // wrapped private key. See RFC 8410 §7 (CurvePrivateKey ::=
  // OCTET STRING) for the exact ASN.1 framing.
  const pkcs8 = new Uint8Array(await subtle.exportKey("pkcs8", kp.privateKey));
  if (pkcs8.length < 48) {
    throw new Error("ensureLocalIdentity: PKCS8 export too short");
  }
  const seed = pkcs8.slice(16, 48);
  // SPKI export — Ed25519 public key sits at offset 12..44. RFC 8410 §4
  // pins the ASN.1 header at exactly 12 bytes for Ed25519.
  const spki = new Uint8Array(await subtle.exportKey("spki", kp.publicKey));
  if (spki.length < 44) {
    throw new Error("ensureLocalIdentity: SPKI export too short");
  }
  const pub = spki.slice(12, 44);
  // Solana keypair: seed (32) || pubkey (32). Pack into a plain
  // number[] so `chrome.storage.local.set` JSON-serialises it cleanly.
  const secret = new Array<number>(64);
  for (let i = 0; i < 32; i++) secret[i] = seed[i] ?? 0;
  for (let i = 0; i < 32; i++) secret[32 + i] = pub[i] ?? 0;
  return {
    pubkey_base58: base58Encode(pub),
    secret,
  };
}
