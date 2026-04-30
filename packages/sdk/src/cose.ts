// COSE_Sign1 wrapper — used ONLY by `client.ts::signMemory`.
//
// `coseSignPayload(canonicalCbor, keypairJson)` calls WASM `sign_cose_payload`
// which:
//   1. takes the canonical-CBOR bytes verbatim (the server built them, so
//      re-deriving in JS would produce a different byte sequence and break
//      content_integrity verification),
//   2. wraps them in a COSE_Sign1 envelope with `alg=EdDSA, kid=<pubkey>`,
//   3. signs `Sig_structure1(protected, payload)` per RFC 8152.
//
// This is intentionally NOT exposed via the `Signer` interface — that
// interface is for raw byte-signers (Decision 4). COSE is one specific
// signature *envelope*, applicable only to the sign-memory flow.

import { UserError } from "./errors.js";
import type { KeypairJson } from "./keypair.js";
import { loadWasm } from "./wasm.js";

/**
 * Wrap server-built canonical-CBOR bytes in a COSE_Sign1 envelope signed
 * by the keypair encoded in `keypairJson`.
 *
 * @param canonicalCbor - Non-empty canonical-CBOR payload, exactly as
 *                        returned by the server's `/api/pending/...`
 *                        endpoint. Do NOT re-encode in JS — any drift
 *                        breaks `content_integrity`.
 * @param keypairJson   - The Ed25519 keypair (JSON shape) used to sign.
 * @returns The COSE_Sign1 envelope bytes, ready to base64-encode and POST
 *          to `/api/sign-callback`.
 * @throws `UserError` on empty / non-`Uint8Array` input, on any WASM
 *         failure, or if the WASM-emitted envelope does not start with
 *         the expected CBOR-array prefix `0x84`.
 */
export async function coseSignPayload(
  canonicalCbor: Uint8Array,
  keypairJson: KeypairJson
): Promise<Uint8Array> {
  if (!(canonicalCbor instanceof Uint8Array)) {
    throw new UserError("coseSignPayload: canonicalCbor must be a Uint8Array");
  }
  if (canonicalCbor.length === 0) {
    throw new UserError("coseSignPayload: refusing to sign empty payload");
  }
  const wasm = await loadWasm();
  let cose: Uint8Array;
  try {
    cose = wasm.sign_cose_payload(canonicalCbor, keypairJson);
  } catch (e) {
    throw new UserError(`sign_cose_payload failed: ${describeError(e)}`, e);
  }
  if (!(cose instanceof Uint8Array)) {
    throw new UserError(
      "internal: WASM sign_cose_payload did not return Uint8Array"
    );
  }
  // The COSE_Sign1 envelope is a CBOR-encoded 4-element array — first byte
  // 0x84. A different first byte means the WASM build drifted and we must
  // surface that loudly rather than POST garbage to the server.
  if (cose.length === 0 || cose[0] !== 0x84) {
    throw new UserError(
      `internal: COSE_Sign1 envelope must start with 0x84, got 0x${
        cose[0]?.toString(16) ?? "00"
      }`
    );
  }
  return cose;
}

function describeError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return JSON.stringify(e);
}
