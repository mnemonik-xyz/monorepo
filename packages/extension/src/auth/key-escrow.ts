// Key-escrow client (T17). Implements Decision 9 on the client side:
// a passphrase-derived Argon2id key wraps the Ed25519 secret in an
// AES-GCM-256 envelope. The server (T15, `mcp/src/escrow.rs`) holds
// the opaque ciphertext + KDF params keyed by `google_sub`; it never
// sees the passphrase OR the plaintext key. The wrong-passphrase
// detection is local (AES-GCM tag mismatch on `decrypt`), so the
// extension does NOT spend its 5/24h server rate-limit budget on
// dictionary attacks the user triggers themselves.
//
// THREAT MODEL — what this module defends against
//
//   - server compromise              → only ciphertext + KDF params leak;
//                                      offline brute force is bounded by
//                                      the Argon2id parameters below.
//   - lost device                    → user re-derives on a new device with
//                                      their passphrase + the server-stored
//                                      blob. Restore-rate is 5/24h/google_sub
//                                      so online brute force is bounded.
//   - stolen Google account          → attacker would also need the user's
//                                      passphrase. PUT-side `pubkey_base58`
//                                      binding (T15) closes the swap-key
//                                      attack.
//
// THREAT MODEL — what this module does NOT defend against
//
//   - filesystem compromise of the user's profile (chrome.storage.local
//     is plaintext at rest on disk — see `src/auth/session.ts` for the
//     full caveat).
//   - keylogger / live RAM dump while the passphrase is in memory.
//   - server colluding with attacker to mint a fake JWT — server JWT
//     signing-key compromise is out of scope.
//
// ARGON2ID PARAMETERS (D9)
//
//   memory_cost = 65 536 KiB (64 MiB)
//   time_cost   = 3 iterations
//   parallelism = 1
//   hash_length = 32 bytes (256-bit AES-GCM key)
//   salt        = 16 random bytes per wrap (never reused)
//
// These hit the OWASP Argon2id minimums for password-storage and
// land in the "≥100ms on a 2024-class laptop" window on
// `hash-wasm`'s wasm-SIMD path. They are documented in this file
// SO the security-auditor can verify them by reading source, and as
// constants below so test code can assert them without re-deriving.
//
// AES-GCM nonces
//
//   12 bytes (96 bits) — the only nonce length the WebCrypto AES-GCM
//   implementation accepts. We generate a fresh `crypto.getRandomValues`
//   nonce per wrap. The (key, nonce) pair MUST be unique per encryption
//   — the random 16-byte salt makes the key fresh too, so the (key,
//   nonce) collision probability is negligible even if the same
//   passphrase is reused.
//
// WIRE FORMAT — matches `mcp/src/escrow.rs::KeyEscrowPutRequest`/
// `KeyEscrowGetResponse`:
//
//   { ciphertext_b64, nonce_b64, kdf: 'argon2id', kdf_params: '{...}',
//     pubkey_base58 }
//
// `kdf_params` is JSON-encoded ON THE WIRE (the server stores it
// opaquely). We serialise `{salt_b64, m, t, p}` here in a stable
// shape; future versions MAY add fields, but MUST NOT remove or
// rename existing ones — old blobs need to remain unwrap-able.

import { argon2id } from "hash-wasm";
import { AuthError } from "./types.js";
import { DEFAULT_SERVER_ORIGIN } from "./google-oauth.js";

// ── Argon2id parameters (D9) ────────────────────────────────────────────────

/** Memory cost in KiB. 65 536 KiB = 64 MiB. OWASP minimum: 19 MiB; we
 *  exceed it ~3.4× to push offline brute force toward ~100ms / try on a
 *  modern laptop. */
export const ARGON2ID_MEMORY_COST_KIB = 65_536;

/** Time cost (iteration count). 3 is the OWASP-suggested floor for
 *  Argon2id with this memory parameter. */
export const ARGON2ID_TIME_COST = 3;

/** Parallelism (lanes). 1 keeps the deriver portable across CPU counts
 *  and matches what `hash-wasm`'s WASM build uses; it also avoids
 *  burning four cores on every onboarding step. */
export const ARGON2ID_PARALLELISM = 1;

/** Derived key length in bytes. 32 = 256 bits → AES-GCM-256. */
export const ARGON2ID_HASH_LENGTH = 32;

/** Random salt length in bytes (D9: 16). */
export const ARGON2ID_SALT_LENGTH = 16;

/** AES-GCM nonce length in bytes. 12 is the only length WebCrypto
 *  accepts; any other value rejects at `encrypt` / `decrypt`. */
export const AES_GCM_NONCE_LENGTH = 12;

/** KDF identifier sent on the wire. Pinned so the server can future-
 *  proof against alternative KDFs (scrypt, etc.) without ambiguity. */
export const KDF_IDENTIFIER = "argon2id" as const;

/** Upper bound on parsed `Retry-After` values (T17-S-02). A hostile
 *  MITM or compromised server returning `Retry-After: 999999999`
 *  (~31 years) would otherwise disable the restore UI for the lifetime
 *  of the extension install. 24h covers every legitimate rate-limit
 *  window the server emits (the GET budget is 5/24h). Any value beyond
 *  this is clamped silently. Applies to BOTH the header path and the
 *  JSON-body `retry_after_secs` fallback. */
export const MAX_RETRY_AFTER_SECONDS = 24 * 60 * 60;

// ── Types ───────────────────────────────────────────────────────────────────

/** Parsed `kdf_params` JSON. All fields REQUIRED — we never produce a
 *  partial blob. */
export interface EscrowKdfParams {
  /** Random salt, base64-standard-encoded. */
  salt_b64: string;
  /** Memory cost in KiB. Mirror of `ARGON2ID_MEMORY_COST_KIB` at wrap-
   *  time; on unwrap we trust the value in the blob so an old blob
   *  written under different params can still be decrypted. */
  m: number;
  /** Time cost / iterations. */
  t: number;
  /** Parallelism. */
  p: number;
}

/** The escrow blob shape — wire-format symmetric to T15's PUT body and
 *  GET response. `kdf_params` is the JSON string of an
 *  {@link EscrowKdfParams}; we keep it stringified to match the server
 *  contract (which stores it opaquely as TEXT). */
export interface EscrowBlob {
  ciphertext_b64: string;
  nonce_b64: string;
  kdf: typeof KDF_IDENTIFIER;
  kdf_params: string;
  pubkey_base58: string;
}

/** Thrown by {@link unwrapSecret} when AES-GCM tag verification fails
 *  — i.e. the passphrase is wrong OR the ciphertext was tampered with.
 *  Surfaces a single distinct type so callers can drive UI counter
 *  (5-wrong-attempts) without inspecting error text.
 *
 *  IMPORTANT: this error is thrown LOCALLY. No network call is made.
 *  The TDD anchor `wrong_passphrase_fails_locally` asserts this — the
 *  rate-limit budget is preserved for genuine restore traffic only. */
export class WrongPassphraseError extends Error {
  public override readonly name = "WrongPassphraseError";
  public constructor(message = "wrong passphrase or tampered ciphertext") {
    super(message);
  }
}

/** Thrown by {@link fetchEscrow} when the server reports 429. Carries
 *  the parsed `Retry-After` (header value preferred; falls back to the
 *  JSON `retry_after_secs` field). */
export class EscrowRateLimitError extends Error {
  public override readonly name = "EscrowRateLimitError";
  public readonly retry_after_seconds: number;
  public constructor(retryAfterSeconds: number) {
    super(`key-escrow rate-limited; retry after ${String(retryAfterSeconds)}s`);
    this.retry_after_seconds = retryAfterSeconds;
  }
}

/** Thrown by {@link fetchEscrow} when the server reports 404 (no blob
 *  exists for the user). Separate from `AuthError` so the restore UI
 *  can distinguish "you have no escrow yet" from "transport failed". */
export class EscrowNotFoundError extends Error {
  public override readonly name = "EscrowNotFoundError";
  public constructor(message = "no key-escrow blob found for this account") {
    super(message);
  }
}

/** Audit S2 / FW-S-02: thrown by `Restore` (and any future caller) when
 *  the encrypted blob fetched from `/api/key-escrow` carries a
 *  `pubkey_base58` that does NOT match the Google-account-linked
 *  `existingPubkey` returned by `/oauth/google/lookup`. The server is
 *  supposed to bind those at PUT time; a mismatch indicates a buggy or
 *  compromised server (or a swapped blob in some pathological proxy
 *  scenario). Either way the unwrap MUST be refused — proceeding would
 *  silently persist an identity whose persisted `pubkey_base58` does
 *  not match the secret decoded from the blob. */
export class EscrowPubkeyMismatchError extends Error {
  public override readonly name = "EscrowPubkeyMismatchError";
  public readonly expectedPubkey: string;
  public readonly blobPubkey: string;
  public constructor(expectedPubkey: string, blobPubkey: string) {
    super(
      `escrow blob pubkey does not match the account-linked pubkey ` +
        `(expected ${expectedPubkey}, blob ${blobPubkey})`,
    );
    this.expectedPubkey = expectedPubkey;
    this.blobPubkey = blobPubkey;
  }
}

// ── Base64 helpers ──────────────────────────────────────────────────────────

/** Encode `bytes` as standard base64 (RFC 4648 §4, `+/` alphabet, `=`
 *  padding). Wire-format symmetric to the Rust server's
 *  `base64::engine::general_purpose::STANDARD`.
 *
 *  Exported (T17-C-06) so dependent components like
 *  `popup/onboarding/SetPassphrase.tsx` reuse the single canonical
 *  encoder rather than duplicating the logic. */
export function bytesToBase64(bytes: Uint8Array): string {
  // btoa over a binary-string is fastest for the small payload sizes we
  // wrap (<= ~100 bytes ciphertext); we don't need a streaming encoder.
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i]!);
  }
  return globalThis.btoa(bin);
}

/** Decode standard base64. Throws `AuthError` on malformed input so the
 *  caller can surface a typed error to the UI. */
function base64ToBytes(b64: string): Uint8Array {
  let bin: string;
  try {
    bin = globalThis.atob(b64);
  } catch (cause) {
    throw new AuthError("invalid base64 in escrow blob", { cause });
  }
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

// ── KDF + key import ────────────────────────────────────────────────────────

/**
 * Derive a 256-bit AES-GCM key from `passphrase` + `salt` using Argon2id
 * with the parameters documented at the top of this module.
 *
 * The derived key material is imported into WebCrypto as a
 * non-extractable `CryptoKey` (`extractable=false`) bound to AES-GCM
 * encrypt+decrypt usages only. This stops accidental keylog via the
 * `crypto.subtle.exportKey` path; the underlying derived bytes are
 * still present on the Argon2id stack until garbage collection — which
 * is the standard limitation of any JS-implemented KDF. T17 cannot
 * close that gap without leaving the browser.
 */
export async function deriveKey(
  passphrase: string,
  salt: Uint8Array,
  params?: { memoryCostKib?: number; timeCost?: number; parallelism?: number },
): Promise<CryptoKey> {
  if (typeof passphrase !== "string" || passphrase.length === 0) {
    throw new AuthError("deriveKey: passphrase must be a non-empty string");
  }
  if (!(salt instanceof Uint8Array) || salt.length === 0) {
    throw new AuthError("deriveKey: salt must be a non-empty Uint8Array");
  }
  const memorySize = params?.memoryCostKib ?? ARGON2ID_MEMORY_COST_KIB;
  const iterations = params?.timeCost ?? ARGON2ID_TIME_COST;
  const parallelism = params?.parallelism ?? ARGON2ID_PARALLELISM;

  // hash-wasm `argon2id` with `outputType: "binary"` returns a Uint8Array
  // of `hashLength` bytes.
  const derived: Uint8Array = await argon2id({
    password: passphrase,
    salt,
    iterations,
    memorySize,
    parallelism,
    hashLength: ARGON2ID_HASH_LENGTH,
    outputType: "binary",
  });

  const key = await crypto.subtle.importKey(
    "raw",
    derived,
    { name: "AES-GCM" },
    /* extractable */ false,
    ["encrypt", "decrypt"],
  );
  // Wipe the raw key bytes immediately. The WebCrypto handle now owns
  // them; nothing else in this module needs them. The wipe does not
  // erase copies the WASM-Argon2id internals may have left on its
  // stack, but it removes the only reference reachable from JS.
  derived.fill(0);
  return key;
}

// ── Wrap / unwrap ───────────────────────────────────────────────────────────

/**
 * AES-GCM-256-encrypt `secret` under a fresh Argon2id-derived key.
 *
 * Inputs:
 *   - `secret`         — the Ed25519 secret bytes (32 or 64; this module
 *                        does not interpret the payload, so any reasonable
 *                        keypair format is acceptable).
 *   - `passphrase`     — user-typed recovery passphrase. Plaintext stays
 *                        on the JS heap only as long as the caller's
 *                        reference does.
 *   - `pubkey_base58`  — the Ed25519 public key. Server (T15) verifies it
 *                        matches the linked owner before persisting.
 *
 * Output is a fully-formed {@link EscrowBlob} ready to PUT.
 */
export async function wrapSecret(
  secret: Uint8Array,
  passphrase: string,
  pubkey_base58: string,
): Promise<EscrowBlob> {
  if (!(secret instanceof Uint8Array) || secret.length === 0) {
    throw new AuthError("wrapSecret: secret must be a non-empty Uint8Array");
  }
  if (typeof passphrase !== "string" || passphrase.length === 0) {
    throw new AuthError("wrapSecret: passphrase must be a non-empty string");
  }
  if (typeof pubkey_base58 !== "string" || pubkey_base58.length === 0) {
    throw new AuthError("wrapSecret: pubkey_base58 must be a non-empty string");
  }

  const salt = crypto.getRandomValues(new Uint8Array(ARGON2ID_SALT_LENGTH));
  const nonce = crypto.getRandomValues(new Uint8Array(AES_GCM_NONCE_LENGTH));

  const key = await deriveKey(passphrase, salt);
  let ciphertext: ArrayBuffer;
  try {
    ciphertext = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: nonce },
      key,
      secret,
    );
  } catch (cause) {
    throw new AuthError("wrapSecret: AES-GCM encrypt failed", { cause });
  }

  const kdfParams: EscrowKdfParams = {
    salt_b64: bytesToBase64(salt),
    m: ARGON2ID_MEMORY_COST_KIB,
    t: ARGON2ID_TIME_COST,
    p: ARGON2ID_PARALLELISM,
  };

  return {
    ciphertext_b64: bytesToBase64(new Uint8Array(ciphertext)),
    nonce_b64: bytesToBase64(nonce),
    kdf: KDF_IDENTIFIER,
    kdf_params: JSON.stringify(kdfParams),
    pubkey_base58,
  };
}

/**
 * Inverse of {@link wrapSecret}. Throws {@link WrongPassphraseError} on
 * AES-GCM tag mismatch (wrong passphrase or tampered ciphertext) —
 * **no network call is made**. The "wrong_passphrase_fails_locally"
 * TDD anchor binds this guarantee.
 *
 * Throws {@link AuthError} on malformed blob (bad base64, wrong KDF id,
 * missing kdf_params fields, etc.) so the caller can distinguish a
 * "server delivered garbage" failure from "user typed the wrong
 * passphrase".
 */
export async function unwrapSecret(
  blob: EscrowBlob,
  passphrase: string,
): Promise<Uint8Array> {
  if (!blob || typeof blob !== "object") {
    throw new AuthError("unwrapSecret: blob must be an object");
  }
  if (blob.kdf !== KDF_IDENTIFIER) {
    throw new AuthError(`unwrapSecret: unsupported KDF '${String(blob.kdf)}'`);
  }
  if (typeof passphrase !== "string" || passphrase.length === 0) {
    throw new AuthError("unwrapSecret: passphrase must be a non-empty string");
  }

  let params: EscrowKdfParams;
  try {
    const parsed = JSON.parse(blob.kdf_params) as unknown;
    if (!parsed || typeof parsed !== "object") {
      throw new Error("kdf_params is not an object");
    }
    const o = parsed as Record<string, unknown>;
    if (
      typeof o.salt_b64 !== "string" ||
      typeof o.m !== "number" ||
      typeof o.t !== "number" ||
      typeof o.p !== "number"
    ) {
      throw new Error("kdf_params shape is wrong");
    }
    params = { salt_b64: o.salt_b64, m: o.m, t: o.t, p: o.p };
  } catch (cause) {
    throw new AuthError("unwrapSecret: kdf_params is not valid JSON", {
      cause,
    });
  }
  const salt = base64ToBytes(params.salt_b64);
  const nonce = base64ToBytes(blob.nonce_b64);
  if (nonce.length !== AES_GCM_NONCE_LENGTH) {
    throw new AuthError(
      `unwrapSecret: nonce length ${String(nonce.length)} != ${String(
        AES_GCM_NONCE_LENGTH,
      )}`,
    );
  }
  const ciphertext = base64ToBytes(blob.ciphertext_b64);

  const key = await deriveKey(passphrase, salt, {
    memoryCostKib: params.m,
    timeCost: params.t,
    parallelism: params.p,
  });
  let plaintext: ArrayBuffer;
  try {
    plaintext = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: nonce },
      key,
      ciphertext,
    );
  } catch {
    // AES-GCM tag mismatch surfaces as an opaque `OperationError`.
    // Translate to the single typed error so the UI counter (5-wrong-
    // attempts) doesn't need to string-match. The original error is
    // dropped intentionally — its message can vary by browser and
    // carries no actionable detail for the user.
    throw new WrongPassphraseError();
  }
  return new Uint8Array(plaintext);
}

// ── Server I/O ──────────────────────────────────────────────────────────────

export interface EscrowHttpOptions {
  serverOrigin?: string;
  fetchImpl?: typeof fetch;
}

function escrowUrl(opts: EscrowHttpOptions | undefined): string {
  const origin = (opts?.serverOrigin ?? DEFAULT_SERVER_ORIGIN).replace(
    /\/+$/u,
    "",
  );
  return new URL("/api/key-escrow", origin).toString();
}

function requireJwt(jwt: string, where: string): void {
  if (typeof jwt !== "string" || jwt.length === 0) {
    throw new AuthError(`${where}: jwt is required`);
  }
}

/**
 * `PUT /api/key-escrow`. Persists the wrapped blob under the user's
 * Google `sub`. Server validates `pubkey_base58` matches the linked
 * pubkey — mismatch surfaces here as `AuthError` (403).
 */
export async function uploadEscrow(
  jwt: string,
  blob: EscrowBlob,
  opts: EscrowHttpOptions = {},
): Promise<void> {
  requireJwt(jwt, "uploadEscrow");
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  let response: Response;
  try {
    response = await fetchImpl(escrowUrl(opts), {
      method: "PUT",
      headers: {
        authorization: `Bearer ${jwt}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(blob),
    });
  } catch (cause) {
    throw new AuthError("uploadEscrow: server unreachable", { cause });
  }
  if (response.status === 401) {
    throw new AuthError("uploadEscrow: server rejected JWT (401)");
  }
  if (response.status === 403) {
    throw new AuthError("uploadEscrow: server rejected pubkey binding (403)");
  }
  if (response.status === 429) {
    // PUT is not currently rate-limited on the server, but we surface
    // 429 as a typed error anyway so future server-side throttling
    // does not require a client change.
    throw new EscrowRateLimitError(await parseRetryAfter(response, 60));
  }
  if (!response.ok) {
    throw new AuthError(
      `uploadEscrow: server returned ${String(response.status)}`,
    );
  }
}

/**
 * `GET /api/key-escrow`. Returns the wrapped blob. Throws:
 *   - {@link EscrowNotFoundError} on 404,
 *   - {@link EscrowRateLimitError} on 429 (with `retry_after_seconds`),
 *   - {@link AuthError} on other non-2xx / transport failures /
 *     malformed-body responses.
 */
export async function fetchEscrow(
  jwt: string,
  opts: EscrowHttpOptions = {},
): Promise<EscrowBlob> {
  requireJwt(jwt, "fetchEscrow");
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  let response: Response;
  try {
    response = await fetchImpl(escrowUrl(opts), {
      method: "GET",
      headers: { authorization: `Bearer ${jwt}` },
    });
  } catch (cause) {
    throw new AuthError("fetchEscrow: server unreachable", { cause });
  }
  if (response.status === 401) {
    throw new AuthError("fetchEscrow: server rejected JWT (401)");
  }
  if (response.status === 404) {
    throw new EscrowNotFoundError();
  }
  if (response.status === 429) {
    throw new EscrowRateLimitError(await parseRetryAfter(response, 3600));
  }
  if (!response.ok) {
    throw new AuthError(
      `fetchEscrow: server returned ${String(response.status)}`,
    );
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    throw new AuthError("fetchEscrow: response is not valid JSON", { cause });
  }
  if (!body || typeof body !== "object") {
    throw new AuthError("fetchEscrow: response is not an object");
  }
  const o = body as Record<string, unknown>;
  if (
    typeof o.ciphertext_b64 !== "string" ||
    typeof o.nonce_b64 !== "string" ||
    typeof o.kdf !== "string" ||
    typeof o.kdf_params !== "string" ||
    typeof o.pubkey_base58 !== "string"
  ) {
    throw new AuthError("fetchEscrow: response shape is wrong");
  }
  if (o.kdf !== KDF_IDENTIFIER) {
    throw new AuthError(`fetchEscrow: unsupported KDF '${o.kdf}'`);
  }
  return {
    ciphertext_b64: o.ciphertext_b64,
    nonce_b64: o.nonce_b64,
    kdf: KDF_IDENTIFIER,
    kdf_params: o.kdf_params,
    pubkey_base58: o.pubkey_base58,
  };
}

/**
 * `DELETE /api/key-escrow`. Hard-delete; server treats absent-row as
 * 200 (idempotent revocation per D9).
 */
export async function deleteEscrow(
  jwt: string,
  opts: EscrowHttpOptions = {},
): Promise<void> {
  requireJwt(jwt, "deleteEscrow");
  const fetchImpl = opts.fetchImpl ?? globalThis.fetch.bind(globalThis);
  let response: Response;
  try {
    response = await fetchImpl(escrowUrl(opts), {
      method: "DELETE",
      headers: { authorization: `Bearer ${jwt}` },
    });
  } catch (cause) {
    throw new AuthError("deleteEscrow: server unreachable", { cause });
  }
  if (response.status === 401) {
    throw new AuthError("deleteEscrow: server rejected JWT (401)");
  }
  if (!response.ok) {
    throw new AuthError(
      `deleteEscrow: server returned ${String(response.status)}`,
    );
  }
}

/**
 * Rotate the passphrase: fetch blob → unwrap with `oldPassphrase` →
 * re-wrap under `newPassphrase` (fresh salt + nonce) → upload.
 *
 * On wrong old passphrase, propagates {@link WrongPassphraseError}
 * before any second network call — preserves rate-limit budget AND
 * leaves the server-side blob unchanged.
 */
export async function rotatePassphrase(
  jwt: string,
  oldPassphrase: string,
  newPassphrase: string,
  opts: EscrowHttpOptions = {},
): Promise<void> {
  requireJwt(jwt, "rotatePassphrase");
  if (
    typeof oldPassphrase !== "string" ||
    oldPassphrase.length === 0 ||
    typeof newPassphrase !== "string" ||
    newPassphrase.length === 0
  ) {
    throw new AuthError("rotatePassphrase: passphrases must be non-empty");
  }
  if (oldPassphrase === newPassphrase) {
    throw new AuthError(
      "rotatePassphrase: new passphrase must differ from old",
    );
  }
  const blob = await fetchEscrow(jwt, opts);
  const secret = await unwrapSecret(blob, oldPassphrase);
  try {
    const rewrapped = await wrapSecret(
      secret,
      newPassphrase,
      blob.pubkey_base58,
    );
    await uploadEscrow(jwt, rewrapped, opts);
  } finally {
    // Best-effort wipe of the recovered secret bytes — minimises the
    // window in which an attacker who already has live-heap access can
    // pull the Ed25519 secret out. JS gives no guarantee here; this is
    // hygiene, not a security boundary.
    secret.fill(0);
  }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Parse `Retry-After` (header — preferred) then fall back to JSON
 * `retry_after_secs`, then to `fallback`. Defensive against either
 * channel being absent or non-integer.
 *
 * The function is async because the JSON body fallback requires reading
 * the response stream; we clone the response first so the caller can
 * still read the original body afterward without "Body already
 * consumed" errors.
 */
async function parseRetryAfter(
  response: Response,
  fallback: number,
): Promise<number> {
  const clamp = (n: number): number =>
    Math.min(Math.max(1, Math.floor(n)), MAX_RETRY_AFTER_SECONDS);

  const headerVal = response.headers.get("retry-after");
  if (headerVal !== null) {
    const n = Number.parseInt(headerVal, 10);
    if (Number.isFinite(n) && n > 0) return clamp(n);
  }
  // Body fallback: the server (T15 `escrow.rs::rate_limited`) emits
  // BOTH a `Retry-After` header AND a JSON body of shape
  // `{retry_after_secs: <int>}`. The server has a degenerate path
  // (`HeaderValue` construction failure → header omitted, body still
  // sent) — covering it here keeps the typed error informative even
  // when the header channel drops. Per T17-S-02, the parsed body value
  // is bounded too, so a server JSON of `{retry_after_secs: 999999999}`
  // cannot disable the restore UI longer than `MAX_RETRY_AFTER_SECONDS`.
  try {
    const clone = response.clone();
    const body = (await clone.json()) as unknown;
    if (body && typeof body === "object") {
      const n = (body as Record<string, unknown>).retry_after_secs;
      if (typeof n === "number" && Number.isFinite(n) && n > 0) {
        return clamp(n);
      }
    }
  } catch {
    // Body is not JSON, already consumed, or server returned an empty
    // body. Fall through to the hardcoded floor.
  }
  return clamp(fallback);
}
