# mnemonik-verify

## Purpose

Confirm that a specific attestation is genuine: the server fetches the original bytes from Arweave, the content hash matches, and the COSE_Sign1 signature is valid against the claimed signer. Arweave is the single record of an anchored memory. Use when the user wants cryptographic certainty about a recalled memory, or before relying on a third-party-claimed attestation.

## Trigger

**Positive examples (DO use):**

- The user just received an attestation_id from another party and asks "is this real" or "verify this".
- The user is about to act on a recalled memory whose authenticity matters (e.g., a recorded decision is being cited as authority) — verify it first.
- A recall result has a low score AND the user wants to confirm the row is at least cryptographically intact even if not a great semantic match.
- An `anchored` attestation needs to be confirmed as actually stored — the Arweave read-back is what `verify` produces.

**Negative examples (DO NOT use):**

- The user just attested something themselves seconds ago — the sign_memory response already confirms it was stored. Re-verification is redundant.
- Every recall result — verifying every row on every recall is wasteful. Only verify when authenticity actually matters for the next action.
- The user is debugging a hash mismatch they already understand — surface the mismatch, do not loop on verify.
- Local-mode rows that have synthetic `local:` tx IDs — verify cannot reach Arweave for them; it can only rebuild the local content hash.

## Context to gather

- The `arweave_tx` of the attestation.
- Whether the row is local-mode or anchored. Local-mode verify only rebuilds the content hash; anchored verify fetches the Arweave bytes and checks the signature.

## Tool

Underlying MCP tool: `mnemonic_verify`.

Arguments:

- `arweave_tx` (string, required) — Arweave transaction ID.

Returns whether the signature over the Arweave bytes is valid and the recomputed content hash matches.

## Guardrails

- Verify is a read-only operation. It never modifies state.
- Do not treat a verify failure as "the user is lying" — it could be a corrupted row, a network problem reaching Arweave, or a mismatch between local cache and chain. Surface the failure reason from `data` rather than asserting bad faith.
- For local-mode rows (`arweave_tx` starts with `local:`), verify confirms only the content hash, not Arweave storage — say so in the response.
- Do not verify the same `attestation_id` repeatedly in a session unless something changed. The result is deterministic for a given row.
