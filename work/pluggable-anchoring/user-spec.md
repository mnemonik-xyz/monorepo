# User spec: pluggable-anchoring

**Feature:** pluggable-anchoring
**Issue:** #70 (`erc8004-0`, "Anchor pluggability, Phase 3α")
**Status:** drafting

---

## What this feature does

Today every anchored write commits a hash to the Solana blockchain via an SPL Memo transaction. That path is hardcoded: there is no way to turn it off, switch chains, or skip it without deleting code.

This feature introduces an `AnchorWriter` trait with a selection mechanism (config value) so that the anchor is a **choice**, not a constant. Three choices ship:

| Selection | Behaviour |
|---|---|
| `anchor_type = "solana"` (default) | Existing SPL Memo path. Nothing changes for an operator who does not touch config. |
| `anchor_type = "none"` | No memo is written. The write is local to the server. No chain interaction, no lamport cost. |
| `anchor_type = "ethereum"` | Stub only. Compiles and selects, but the implementation is a no-op that returns an empty reference. A real client ships in issue #75. |

Switching between them is a **config change and a restart**, not a code change and a deploy.

---

## Why

### Anchor::None — config change, not code deletion

`work/chain-agnostic/` stage 2 proposes "stop writing the memo". Its open question Q-1 asks: should that be behind an environment flag so it can be re-enabled without reverting a deploy?

`Anchor::None` is the answer. Choosing it is "stop writing the memo". Choosing `SolanaAnchor` restores it. No dead code path survives, and no second mechanism (boolean env flag + conditional) is needed. More importantly, the switch is **reversible on a deploy** rather than requiring a code revert.

### ERC-8004 path

In an ERC-8004-routed attestation, the `validationResponse` transaction is itself the anchor — the on-chain `responseHash` already commits the data. That path gains anchoring for free; there needs to be somewhere to record an externally-supplied anchor reference. `AnchorWriter` + `AnchorRecord` is that place.

### Safety gate (unchanged)

`work/chain-agnostic/` D-1 still applies: enumeration (listing an owner's anchored memories) must be measured as working before any production deployment selects `Anchor::None`, because the Solana memo is currently the only thing that can list an owner's ledger. Pluggability makes the switch reversible; it does not make it safe to flip prematurely.

---

## Scenarios

### Scenario 1 — operator keeps today's behaviour

Operator does not change `anchor_type` in their config (or it is absent). The default value `"solana"` selects `SolanaAnchor`. Every anchored write produces an SPL Memo on Solana. Behaviour is identical to today.

### Scenario 2 — operator selects Anchor::None

Operator sets `anchor_type = "none"` and restarts. Anchored writes complete without a Solana RPC call: no memo is sent, `solana_tx` stores an empty string (same as the existing local-mode synthetic id behaviour). The row is persisted; Arweave storage still proceeds if configured. The operator can revert to `"solana"` at any time with another config change.

The memo-reader code (`SolanaClient::list_memo_anchors`, `read_memo`) is not removed. Future reads of historical rows with real Solana signatures continue to work.

### Scenario 3 — future operator selects EthereumAnchor (stub)

Operator sets `anchor_type = "ethereum"`. The stub `EthereumAnchor` is selected; it returns an `AnchorRecord` with an empty reference. No Ethereum client is exercised in this release. This selection exists so the type system and the config parser are ready for issue #75 without any future breaking change to the trait or the DB schema.

---

## What this feature does NOT do

- Remove `core/src/solana/mod.rs` or any of its memo-reader functions. The reader lives forever per `work/chain-agnostic/` D-2 so historical rows remain recoverable.
- Add a real Ethereum client or any on-chain Ethereum interaction. That is issue #75.
- Change any storage backend. Storage (where bytes live) and anchoring (where the hash is committed) are separate axes per `work/pluggable-storage/`. The `solana_tx` column is not renamed; two new columns are added additively and legacy rows are read back as `Anchor::Solana(..)`.
- Change the `Signer` seam. Anchoring is about publication of a hash, not about who signs.
- Change the Arweave upload path.
- Change any API surface or response shape visible to end users or MCP clients.
