# Decisions — agent-identity-records

Append-only log. Each entry: date, who, what, why, what changes downstream.

---

## 2026-10-05 — Self-signed records, Mnemonik as index

Author: claude, approved in discussion by the owner ("Mnemonik as a source of
identity — let's go with it").

Options compared:

- A central registry (database) with an MCP "register" tool: rejected. The
  operator becomes a trust anchor that can substitute keys, and an LLM-callable
  register tool would give the agent the owner's signing power.
- A registry smart contract (for example ERC-8004): kept as a planned mirror. It
  needs an EVM account and gas per agent and the dual-key binding first, and the
  swap involves non-EVM chains.
- Self-signed record chains anchored on Arweave and indexed by Mnemonik: chosen.
  Self-certifying identifiers, no new authority, and it reuses the shipped
  anchoring and enumeration.

Key rules: pre-rotation (`next_key_hash`); rotation beats update on the same
`prev`; duplicity makes an identity `conflicted`; only the owner publishes; the
MCP surface is read-only.

Downstream: closes gap M4 (key rotation) and the recipient discovery gap in
`work/agentic-swap/`; `docs/sealed-a2a.md` recipient discovery (#74) is
superseded by resolution by `id` once shipped.

---

## 2026-10-05 — Revisions after the adversarial review

Author: claude, from a review workflow with a skeptic per lens (21 confirmed
findings on this folder).

- `id` is the blake3 of the seq 0 JCS without `id`, so it commits to the first
  key and the first `next_key_hash`. A second seq 0 is a different identity.
- `prev` hashes the signed payload, not the COSE bytes, because unprotected
  header labels are not signed. Extra unprotected labels are rejected.
- Records use a distinct protected content type, so a signature from another
  Mnemonik flow (legacy HTTP signing) never verifies as a record.
- Fork rules work on branches: a branch with a rotation beats branches without
  one; two rotation branches mean `conflicted`. Update revocations are
  provisional; rotation revocations are final.
- New status `incomplete` for hidden records and discovery limits. The SDK keeps
  the highest verified head (rollback guard). `valid_until` is at most 90 days.
- The server checks authorization against a known parent, does not require the
  JWT subject to equal the signer, and charges the JWT subject.
- AgentCards in records carry a JWS by the record key and must match `key` and
  `enc_key`. `accounts` stays planned until `KEY_BINDING_V1` ships.
- Old signatures are judged by Arweave anchor time, not `created_at`.
- Stdio publish is out of version 1. Tool docs and counts move to task 2.

## Codex review on #277 (2026-10-07)

- The MCP resolver returns the signed records with their anchor times; the
  server's resolved view is a hint. Otherwise an MCP-only caller trusts the
  index for the key and the card.
- `created_at` is bounded by the anchor time (at most 10 minutes later), so a
  future-dated record cannot stretch the 90-day expiry of a stolen key.
- `publish` in task 3 has no `accounts` until `KEY_BINDING_V1` ships.
