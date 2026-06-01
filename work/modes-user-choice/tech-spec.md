---
created: 2026-06-01
status: draft
branch: claude/modes-user-choice-Qkk6X
size: L
related:
  - work/modes-user-choice/user-spec.md
---

# Tech Spec: Mode as a user choice (intent-driven), not a server parameter

## Problem recap

`STORAGE_MODE` / `PAYMENT_MODE` are read once in `mcp/src/config.rs::from_env`
and threaded as `&str` into every tool (`whoami`, `sign_memory`, `recall`,
`verify`) and into the dispatch + paywall in `mcp/src/mcp.rs`. The operator
picks one value for the whole process; the caller cannot choose. CLAUDE.md
encodes the consequence as a hard rule: *"Mode is set at startup, not per-call.
Never mix in one DB."* This spec deliberately revisits that rule.

The user's framing reduces the surface to a single decision per write:

- **local** — artifact stays on the user's own filesystem/self-hosted store.
  Free, offline, private. Protocol-guaranteed free path (whitepaper §5.7.1).
- **participate** — artifact is shared/exchanged with the network. Requires
  durability **and a delivery guarantee**; this is the paid service-layer path.

So "mode" stops being an operator env var and becomes a per-write **intent**
that defaults to `local`.

## Current plumbing (what we touch)

- `mcp/src/config.rs` — `storage_mode`, `payment_mode` fields + env reads.
- `mcp/src/mcp.rs` — `McpState.storage_mode` / `.payment_mode`; the paywall gate
  at `mcp.rs:424` (`is_sign_memory && payment_mode != "none" && storage_mode != "local"`);
  the per-tool dispatch passing `&state.storage_mode` into `tools::*`.
- `mcp/src/tools.rs` — `whoami(.., storage_mode)`, `sign_memory(.., storage_mode)`,
  `sign_memory_inline` (the `if storage_mode == "local"` branch at `tools.rs:332`
  that chooses synthetic `local:` tx ids vs real Arweave+Solana writes, and the
  `if storage_mode != "local"` cost-recording branch at `tools.rs:367`),
  `verify(.., storage_mode)` / `verify_local`.
- `mcp/src/payment.rs` — paywall + `record_attestation_cost`.
- `core/src/storage/sqlite.rs` — `save_attestation`, schema.

## Design

### 1. Mode becomes a per-call parameter with an operator-set policy envelope

Introduce an explicit enum (no more stringly-typed `&str` threading):

```rust
// core/src/storage/mode.rs  (new — pure type, no I/O)
pub enum WriteMode { Local, Participate }
```

- `mnemonic_sign_memory` gains an optional input field `mode: "local" |
  "participate"` (default `local`). This is the per-write intent.
- The operator keeps a *policy envelope* in config — not a single mode, but the
  set of modes the operator is willing/able to serve:
  - `allow_participate: bool` (derived: true iff the server has a funded keypair
    + Arweave/Solana wired). A pure-local operator advertises `local` only.
  - Existing `payment_mode` is reinterpreted: it governs **how** a `participate`
    write is paid, and is irrelevant to `local` writes (which are always free).
- If a caller requests `participate` on a server that only supports `local`, the
  tool returns a typed, actionable error (not a silent downgrade) — the user
  chose to participate; silently storing locally would break the delivery
  promise they asked for.

`whoami` advertises the envelope (`supported_modes`, `default_mode`,
`participate_cost`) so a client can discover what a server offers before writing.

### 2. The free/paid fork follows the mode, per-write

Replace the boot-time paywall condition. Today: `payment_mode != "none" &&
storage_mode != "local"`. New: the gate fires **iff the resolved write mode is
`Participate`**. `Local` writes never touch `payment.rs`. This makes "personal
memory is free" a structural property of the code path, not an operator setting.

### 3. Delivery guarantee — the core new mechanism (see §Delivery options)

`participate` is not "wrote to Arweave". It must produce a verifiable
**delivery receipt**. Detailed options below; the receipt is persisted on the
attestation row and surfaced in `verify`.

### 4. Storage invariant — DECISION DEFERRED to §Open decisions

Whether `local` and `participate` artifacts share one SQLite (tagged by a
`write_mode` column) or live in separate stores is the load-bearing open
decision. Both are sketched below with a recommendation; final call is the
user's per user-spec.

## Delivery guarantee — options (the heart of "participate")

The user's key insight: locally-stored artifacts are *risky to share* because
they may be unreachable when a peer asks. "Participate" must convert a write into
something the protocol can guarantee is **deliverable**, and prove it. Three
candidate definitions of "delivered", in increasing strength:

- **D1 — Durable-anchor receipt (minimum).** A `participate` write is "delivered"
  when (a) COSE bytes are stored at a durable, content-addressed URL (Arweave tx)
  AND (b) the blake3 hash is anchored with an immutable timestamp (Solana memo),
  AND (c) we *read back* the Arweave bytes and re-verify hash+signature before
  declaring success. Receipt = `{arweave_tx, solana_tx, anchor_inclusion_proof,
  readback_verified_at}`. Failure of any step ⇒ write reported as `local` +
  refund (no charge). Cheap, fully objective, no second party required. This is
  the recommended V1.
- **D2 — Recipient ack (point-to-point exchange).** For directed exchange (agent
  A hands an artifact to agent B), delivery = D1 **plus** a signed ACK from B's
  keypair over the content hash. Stronger but needs an online counterparty and a
  handshake — overlaps with the A2A bridge (`work/a2a-bridge/`) and capability
  tokens. Defer to a follow-up; design the receipt struct to carry an optional
  `acks: [{pubkey, sig, at}]` so we don't migrate later.
- **D3 — Availability SLA / replication proof.** Multi-replica storage with
  periodic proof-of-availability challenges. Out of scope; named only so the
  receipt schema reserves room.

**Recommendation:** ship **D1** as the delivery guarantee for V1. It is fully
local-verifiable, needs no counterparty, and directly addresses the stated risk
("artifact unreachable when a peer asks") by guaranteeing a durable, re-read,
anchored copy exists independent of the user's machine. Schema is forward-shaped
for D2/D3.

The concrete code delta vs today's `full` path: today `sign_memory_inline`
writes Arweave+Solana and trusts them. D1 adds the **read-back + re-verify** step
and packages the result as an explicit `delivery_receipt` rather than two loose
tx-id strings. That read-back is the line between "we wrote it somewhere" and
"the protocol guarantees it's retrievable".

## Storage invariant — options for "Never mix in one DB"

- **Option S1 — Tag rows in one DB (recommended).** Add `write_mode TEXT NOT
  NULL DEFAULT 'local'` (and the `delivery_receipt` columns) to the attestations
  table via an idempotent migration; legacy rows = `local`. `recall` works across
  both transparently (cosine search doesn't care about mode). Pro: one identity,
  one DB, local + shared memory coexist for the same user exactly as user-spec
  requires; recall spans both. Con: the old "never mix" invariant is *deliberately
  retired* — CLAUDE.md must be updated in the same PR with the rationale.
- **Option S2 — Separate DB per mode.** `attestations.db` (local) +
  `attestations-shared.db` (participate); router opens the right one per write.
  Pro: preserves the literal invariant; clean blast-radius separation. Con:
  `recall` must fan-out + merge across two stores (new code, ranking across two
  indexes); a single user's memory is split, which fights the user-spec's "coexist
  for one user" goal.

**Recommendation:** **S1**. The user explicitly wants local + shared memory to
coexist for one user and recall to see both; S1 delivers that with a single
migration, while S2 reintroduces the split the user is trying to remove. Retiring
"Never mix in one DB" is then a *conscious* spec change (update CLAUDE.md ## and
`work/.../decisions.md`), not an accident. If the user prefers the conservative
path, S2 is the fallback.

## Research outcome (deep-research 2026-06-01 — see research.md)

The two semantic questions the user flagged for research now have evidence-backed
recommendations, both pulling the same way:

- **"Participate" = broadcast-publish a verifiable public record** (ERC-8004
  reputation/validation analog — the one broadcast pattern actually shipping in
  trustless agent infra), **not** recipient-ACK handoff. Cross-operator *exchange*
  is dominated by directed message-passing that shares no memory (A2A "Opaque
  Execution"); verifiable cross-operator *shared memory* is still aspirational.
  ⇒ V1 `participate` = "anchor durably + make discoverable/verifiable by anyone."
  Directed exchange (recipient ACK) deferred to the A2A bridge.
- **Delivery guarantee = D1** confirmed. ERC-8004 commits hash+URI but explicitly
  does **not** guarantee the off-chain content is retrievable — that gap is
  Mnemonic's wedge, and D1's read-back is the cheap proof that fills it:
  *"ERC-8004 proves a hash; Mnemonic proves the bytes are actually retrievable."*

## Open decisions (need user sign-off before Wave 1)

1. **Storage invariant:** S1 (tag rows, recommended) vs S2 (separate DBs).
2. **Delivery definition for V1:** **D1** (durable-anchor + read-back) —
   research-confirmed; recipient-ACK (D2) deferred (needs online counterparty).
3. **Participate semantics:** **broadcast-publish** (research-recommended) vs
   directed handoff. V1 = broadcast; directed via A2A bridge later.
4. **Mode granularity:** per-request field on `sign_memory` (recommended) vs
   per-identity default persisted server-side vs both.

## Tasks / waves (provisional — finalized after open decisions)

- **Wave 1 — types + config envelope.** `WriteMode` enum in `core`; reinterpret
  config (`allow_participate`, keep `payment_mode` as participate-payment policy);
  `whoami` advertises envelope. No behavior change to existing `full` operators
  yet. Files: `core/src/storage/mode.rs`, `mcp/src/config.rs`, `mcp/src/tools.rs`
  (whoami).
- **Wave 2 — per-call mode + free/paid fork.** `sign_memory` accepts `mode`;
  paywall gate keys off resolved `WriteMode::Participate`; unsupported-mode error.
  Files: `mcp/src/mcp.rs`, `mcp/src/tools.rs`, `mcp/src/payment.rs`.
- **Wave 3 — delivery receipt (D1).** Read-back + re-verify; `delivery_receipt`
  on the row; `verify` surfaces it. Storage migration per chosen S-option.
  Files: `mcp/src/tools.rs`, `core/src/storage/sqlite.rs`.
- **Wave 4 — docs + invariant retirement.** Update CLAUDE.md (retire/rephrase
  "Never mix in one DB" + "Mode is set at startup"), `.env.example`, whitepaper
  §5.7 cross-ref. Append `decisions.md`.
- **Audit waves** (code/security/test) read-only per repo workflow.

`mcp/src/tools.rs`, `mcp/src/mcp.rs`, `mcp/src/config.rs` are the known
parallel-conflict files — waves are sequenced so only one wave touches each.

## Testing

- Unit: `WriteMode` parse/default; paywall fires on `Participate` only, never on
  `Local`; unsupported-mode error on local-only server.
- Integration: `local` write produces synthetic `local:` ids + zero cost; a
  `participate` write produces a `delivery_receipt` whose read-back hash matches;
  forced Arweave read-back failure ⇒ reported `local` + no charge (refund path).
- Recall spans both modes (S1) or merges two stores (S2) — same top-K result set.
- `MockEmbedder` in `#[cfg(test)]` (no `HashEmbedder`, per CLAUDE.md rule 4).
