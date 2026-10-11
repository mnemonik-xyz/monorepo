# Decisions log: receipts

Append-only. Owner decisions, task reports and audit findings go here.

---

## Fixed inputs (owner discussion, 2026-10-11)

- mem0 is a memory engine (extraction, retrieval quality, benchmarks). Mnemonic is a memory
  envelope (signing, hashing, recovery). The two do not compete on the same axis.
- The product sentence is "mem0 remembers; Mnemonic proves what it remembered".
- Receipts hold hashes only. Memory text never enters a receipt.
- The write path has no network call, no payment, no quota and no login.
- The first engine is mem0 self-hosted (`Memory`, `AsyncMemory`). The platform client follows.

## Design decisions (details in `tech-spec.md` §3)

D-1 hashes only · D-2 hook the history writer plus a `ContextVar` scope · D-3 ledger chain and
scope chain · D-4 `content_hash` = blake3 of the raw text · D-5 receipt id = blake3 of the
envelope · D-6 integer timestamps · D-7 `scope_id` by plain deterministic CBOR · D-8 fail
closed with deferred raise · D-9 SQLite store, JSON Lines export · D-10 pure-Python signer,
Rust reference verifier · D-11 no network in the write path · D-12 engine pin and surface check.

## Open questions for the owner

- **Q-1** Package name. `mnemonik` on PyPI is the proposal. Confirm availability.
- **Q-2** Location. `python/mnemonik/` in this repository (proposal), or a separate repository.
  The npm workspace glob `packages/*` is a reason not to put Python under `packages/`.
- **Q-3** Default `on_error`. Proposal: `raise` (an audit layer that skips silently is not one).
- **Q-4** Salted content hashes. Proposal: plain hash in v1, salt mode in v1.1 (`tech-spec.md` §8).
- **Q-5** Upstream path. Proposal: ship in this repository first; open a mem0 issue for an
  `integrations/` folder after the first external users, so the issue can show demand. The
  mem0 gates (accepted label, CLA) apply to that pull request.
- **Q-6** What to freeze while this ships. Proposal: every `work/` initiative that is not
  `receipts`, `docs-actualization` or a bug fix.
