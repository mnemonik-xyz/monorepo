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
