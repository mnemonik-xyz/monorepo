// Internal types shared across the cloud-sync surface (cloud-client +
// cloud-sync drain). Keeping these in a dedicated module lets the
// service-worker and popup runtime depend on the shapes without
// pulling the CloudClient implementation (and `fetch` references) into
// their first-paint bundles.

import type { SourceMeta } from "../store/types.js";

/** Args required by `CloudClient.signRemote`. Mirrors the
 *  `attestation_row` fields the alarm-drain reads off `IndexedDbStore`. */
export interface SignRemoteArgs {
  /** Plaintext memory — D9: cloud-mode users opt into server-visible
   *  content; this is the intended trust boundary. */
  content: string;
  tags: string[];
  source_meta?: SourceMeta;
}

/** Minimal cryptographic-material projection of `AttestationRow`.
 *  The COSE bytes were produced locally during the original
 *  `signMemory`; we re-use them verbatim — D2 forbids the server from
 *  ever holding key material. */
export interface SignerLike {
  cose_bytes: Uint8Array;
  signer_pubkey: string;
}

/** Triple the alarm-drain writes back onto the local row on success. */
export interface SignRemoteResult {
  attestation_id: string;
  solana_tx: string;
  arweave_tx: string;
}
