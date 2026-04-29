// Signer interface + LocalSigner Phase 1 impl.
//
// `Signer` is the byte-signer abstraction (Decision 4). It is **NOT** a
// COSE-envelope signer — COSE wrapping happens in `cose.ts` and is only
// used by `client.ts::signMemory`. Keeping this interface generic means
// future signers (Turnkey HSM, WebAuthn passkeys) plug in without
// re-touching the COSE code path.
//
// Every concrete `Signer` implementation MUST pass the abstract contract
// suite at `test/signer-contract.ts` — that's the only thing that
// guarantees behavioral equivalence across signer impls.

import { UserError } from "./errors.js";
import type { Keypair } from "./keypair.js";
import type { SignerInterface } from "./types.js";
import { loadWasm } from "./wasm.js";

// Re-export the public interface so `import { Signer } from '@mnemonik-xyz/sdk'`
// resolves the type without forcing consumers to know about types.ts.
export type Signer = SignerInterface;

/**
 * In-memory keypair signer. Phase 1 default.
 *
 * Wraps WASM `sign_challenge(keypair_json, bytes)` — that's the existing
 * raw-Ed25519 export. **Do not** use `sign_cose_payload` here: COSE wraps
 * the signature in an envelope which would break the interface contract
 * (callers expect 64-byte raw Ed25519 sigs).
 *
 * Construction holds a reference to the `Keypair`; it does NOT clone the
 * secret. Lifetime of the `LocalSigner` should not exceed the lifetime of
 * the underlying `Keypair`. There is no `dispose()` because JS lacks
 * deterministic destructors — for hard zeroization use a `TurnkeySigner`
 * (Phase 1.5) or an HSM-backed impl instead.
 */
export class LocalSigner implements SignerInterface {
  constructor(private readonly keypair: Keypair) {}

  /** Base58 Ed25519 public key — matches `keypair.pubkey`. */
  get pubkey(): string {
    return this.keypair.pubkey;
  }

  /**
   * Produce a 64-byte raw Ed25519 signature over `bytes`.
   *
   * Empty input is rejected up front — `sign_challenge` would happily sign
   * a zero-length challenge, but the contract suite requires the empty
   * case throw, since signing `b""` is almost always a caller bug.
   */
  async sign(bytes: Uint8Array): Promise<Uint8Array> {
    if (!(bytes instanceof Uint8Array)) {
      throw new UserError("LocalSigner.sign: bytes must be a Uint8Array");
    }
    if (bytes.length === 0) {
      throw new UserError(
        "LocalSigner.sign: refusing to sign zero-length input"
      );
    }
    const wasm = await loadWasm();
    let sig: Uint8Array;
    try {
      // The WASM binding expects keypair_json as a JsValue object — pass
      // the plain `KeypairJson`. `sign_challenge` returns `Uint8Array(64)`.
      sig = wasm.sign_challenge(this.keypair.toJSON(), bytes);
    } catch (e) {
      throw new UserError(`sign_challenge failed: ${describeError(e)}`, e);
    }
    if (!(sig instanceof Uint8Array)) {
      throw new UserError(
        "internal: WASM sign_challenge did not return Uint8Array"
      );
    }
    if (sig.length !== 64) {
      throw new UserError(
        `internal: Ed25519 signature must be 64 bytes, got ${sig.length}`
      );
    }
    return sig;
  }
}

function describeError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return JSON.stringify(e);
}
