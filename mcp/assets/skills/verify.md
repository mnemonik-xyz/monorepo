# mnemonik-verify

## Purpose

Confirm that a specific attestation is genuine: its content hash matches what is anchored on-chain, the COSE_Sign1 signature is valid against the claimed signer, and (for participate-mode rows) the Arweave and Solana records exist and agree. Use when the user wants cryptographic certainty about a recalled memory, or before relying on a third-party-claimed attestation.

## Trigger

**Positive examples (DO use):**

- The user just received an attestation_id from another party and asks "is this real" or "verify this".
- The user is about to act on a recalled memory whose authenticity matters (e.g., a recorded decision is being cited as authority) — verify it first.
- A recall result has a low score AND the user wants to confirm the row is at least cryptographically intact even if not a great semantic match.
- A `participate`-mode attestation needs to be confirmed as actually anchored — the on-chain confirmation is what `verify` produces.

**Negative examples (DO NOT use):**

- The user just attested something themselves seconds ago — the sign_memory response already confirms it was stored. Re-verification is redundant.
- Every recall result — verifying every row on every recall is wasteful. Only verify when authenticity actually matters for the next action.
- The user is debugging a hash mismatch they already understand — surface the mismatch, do not loop on verify.
- Local-mode rows that have synthetic `local:` tx IDs — verify cannot reach a chain for them; it can only confirm the local signature.

## Context to gather

- The `attestation_id`, OR the `solana_tx` and `arweave_tx` pair if the user is verifying a third-party-claimed attestation.
- Whether the row is local-mode or participate-mode. Local-mode verify only checks the signature; participate-mode verify additionally checks chain anchoring.

## Tool

Underlying MCP tool: `mnemonic_verify`.

Arguments:

- `solana_tx` (string) — Solana transaction signature.
- `arweave_tx` (string) — Arweave transaction ID.

Returns whether the recomputed hash matches the on-chain anchor, the signature is valid, and the canonical CBOR roundtrips.

## Guardrails

- Verify is a read-only operation. It never modifies state.
- Do not treat a verify failure as "the user is lying" — it could be a corrupted row, a network problem reaching Arweave, or a mismatch between local cache and chain. Surface the failure reason from `data` rather than asserting bad faith.
- For local-mode rows (`solana_tx` starts with `local:`), verify confirms only the signature roundtrip, not chain anchoring — say so in the response.
- Do not verify the same `attestation_id` repeatedly in a session unless something changed. The result is deterministic for a given row.
