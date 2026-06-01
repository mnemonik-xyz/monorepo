---
created: 2026-06-01
status: approved  # all decisions finalized 2026-06-01; tasks ready (Wave 1)
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

### 4. Storage invariant — DECIDED: S1 (one DB, rows tagged by `write_mode`)

`local` and `participate` artifacts share **one** SQLite, tagged by a `write_mode`
column; recall spans both. This consciously retires CLAUDE.md's "Never mix in one
DB" (update in the same PR). The S1-vs-S2 analysis that led here is kept below as
rationale; the decision is final (see §FINALIZED + decisions.md).

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

**FINALIZED definition (user, 2026-06-01): "anchored AND verified by recall =
delivered."** This is D1's intent, but expressed through the protocol's *own*
read path rather than a bespoke read-back:

1. `participate` write → anchor signed COSE plaintext on Arweave + Solana memo
   timestamp (today's `full` path).
2. **Confirmation = a recall + verify round-trip against the anchored artifact:**
   recall must be able to retrieve it, and `verify` must re-fetch the anchored
   COSE bytes and re-check blake3 hash + Ed25519 signature against the Solana
   anchor (this is exactly what `tools::verify` already does in `full` mode).
3. Only when that round-trip passes is the write reported `participated` and
   charged. If it fails (anchor didn't land / bytes don't verify), the artifact
   stays `local`, no charge — the user is never left believing a still-local
   artifact was delivered.

**Why this framing (user's reframe):** anchored-on-Arweave is "mission done" — the
risk surface is *local-only* artifacts. The single silent failure is claiming
"participated" while the anchor didn't actually land, leaving the artifact local.
"Verified by recall" closes exactly that gap by reusing recall/verify, so there is
**no new bespoke primitive** — just gating the `participated` state + the charge on
a recall/verify round-trip. `delivery_receipt` persists `{arweave_tx, solana_tx,
recall_verified_at}` and is forward-shaped with optional `acks[]` for the deferred
D2 (recipient-ACK via the A2A bridge).

Note: V1 anchors **plaintext** signed bytes (public publish) — `verify` works for
anyone with the tx id, no decryption key. Encrypted-share is explicitly out of
scope (see decisions.md #3).

## Storage invariant — options for "Never mix in one DB" (DECIDED: S1; kept as rationale)

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
`work/.../decisions.md`), not an accident. **S1 is the finalized decision** (user
sign-off 2026-06-01); S2 is recorded only as the rejected conservative alternative.

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

## FINALIZED (2026-06-01, user sign-off) — see decisions.md

1. **Storage invariant → S1** (tag rows in one DB). Retires "Never mix in one DB".
2. **Delivery definition → "anchored AND verified by recall = delivered"** (user's
   words). The proof is a **recall + verify round-trip against the anchored
   artifact**, not a bespoke read-back — see §Delivery below (rewritten). Until it
   passes, the write stays `local` and is not charged.
3. **Participate semantics → broadcast / public publish.** Anchor signed COSE
   **plaintext** (current behavior). No encryption / key-based access in V1.
   Directed/recipient-ACK deferred to the A2A bridge.
4. **Mode granularity → per-request `mode` field** on `sign_memory` (default `local`).
5. **Payment → per shared artifact**; local always free. **Retraction → immutable.**

## Local storage substrate & surfaces (DECIDED — scoped reference, separate build)

The `local` mode must fit **all four surfaces** — CLI, Node-SDK, IDE-hosted agents,
browser extension. These decisions are **finalized** (see decisions.md §"Local
storage substrate", §"Browser store — revised after PAM", §"Bridge mechanism") but
their *implementation* is a **separate build effort, not one of this feature's 8
server-side tasks** — recorded here so the topology is part of the spec.

- **Native surfaces = SQLite, shared canonical DB.** CLI, IDE agents (via the local
  `mnemonic-mcp`), and the Node-SDK all share the server-owned
  `~/.mnemonic/attestations.db` (`rusqlite`, concrete `SqliteStore`, no
  `AttestationStore` trait). Storage analogue of one-keypair-everywhere.
- **Browser = "bridge, else local".** When a local `mnemonic-mcp` is reachable the
  extension uses the **same canonical DB** (real-time, unified). When not, it falls
  back to a **`chrome.storage.local` signed-artifact buffer** — **not** SQLite-WASM,
  **not** an in-browser embedder (offline recall needs an offline embedder, which we
  don't ship; PAM-informed). Offline = list/read buffered artifacts; no semantic
  recall until a bridge returns and the server ingests the buffer.
- **Browser integrity is first-class.** The extension runs **blake3-wasm +
  Ed25519-wasm**, so buffered artifacts are content-addressed and COSE-signed
  identically to native/SDK artifacts (we deliberately do *not* copy PAM's degraded
  SHA-256/unsigned browser tier).
- **Bridge mechanism = Chrome native messaging.** One-time, **per-user, no-admin**
  install (a `mnemonic install-bridge` drops a host manifest in the per-user dir /
  `HKCU`); thereafter Chrome auto-spawns the binary on demand — the user never runs
  a daemon by hand. `localhost` HTTP is a power-user alternative; policy-locked
  machines degrade gracefully to the offline buffer.
- **Convergence = the protocol's job.** The offline split-brain window heals via
  shared Ed25519 identity + `participate`/anchor + the server ingesting the buffer
  on bridge-return — i.e. the same `local → participate` path this feature builds,
  no bespoke local file merge.

## Tasks / waves (FINALIZED — server-side scope only)

- **Wave 1 — types + config envelope.** `WriteMode` enum in `core`; reinterpret
  config (`allow_participate`, keep `payment_mode` as participate-payment policy);
  `whoami` advertises envelope. No behavior change to existing `full` operators
  yet. Files: `core/src/storage/mode.rs`, `mcp/src/config.rs`, `mcp/src/tools.rs`
  (whoami).
- **Wave 2 — per-call mode + free/paid fork.** `sign_memory` accepts `mode`;
  paywall gate keys off resolved `WriteMode::Participate`; unsupported-mode error.
  Files: `mcp/src/mcp.rs`, `mcp/src/tools.rs`, `mcp/src/payment.rs`.
- **Wave 3 — delivery = anchored + verified-by-recall.** After anchor, gate the
  `participated` state + the charge on a **recall + verify round-trip** against the
  anchored artifact (reuse `tools::verify` full-mode path); persist
  `delivery_receipt = {arweave_tx, solana_tx, recall_verified_at}` on the row
  (S1 migration adds `write_mode` + receipt columns). Files: `mcp/src/tools.rs`,
  `core/src/storage/sqlite.rs`.
- **Wave 4 — docs + invariant retirement.** Update CLAUDE.md (retire "Never mix in
  one DB" + "Mode is set at startup" with the conscious-change rationale),
  `.env.example`, whitepaper §5.7 cross-ref. Append `decisions.md`.
- **Audit waves** (code/security/test) read-only per repo workflow.

`mcp/src/tools.rs`, `mcp/src/mcp.rs`, `mcp/src/config.rs` are the known
parallel-conflict files — waves are sequenced so only one wave touches each.

## Testing

- Unit: `WriteMode` parse/default; paywall fires on `Participate` only, never on
  `Local`; unsupported-mode error on local-only server.
- Integration: `local` write produces synthetic `local:` ids + zero cost; a
  `participate` write is reported `participated` only after recall+verify passes
  against the anchored artifact, with a `delivery_receipt`; forced anchor/verify
  failure ⇒ reported `local` + no charge (the "not falsely delivered" guarantee).
- Recall spans both modes (S1) or merges two stores (S2) — same top-K result set.
- `MockEmbedder` in `#[cfg(test)]` (no `HashEmbedder`, per CLAUDE.md rule 4).
