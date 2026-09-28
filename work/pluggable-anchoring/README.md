# pluggable-anchoring — parking lot

> **Order:** this is one step of a larger sequence. See
> [`work/DECOUPLING-SEQUENCE.md`](../DECOUPLING-SEQUENCE.md) for where it sits and what must
> land first.


Status: **TODO — brief, then run user-spec-planning.**

Tracks issue **#70** (`erc8004-0`, "Anchor pluggability (Phase 3α)"), which #69 marks as a
prerequisite or co-requisite of ERC-8004.

Makes the anchor a **choice** rather than a hardcoded Solana memo: Solana becomes one
implementation, Ethereum another, and "none" a third.

Siblings:
- [`work/chain-agnostic/`](../chain-agnostic/) — this folder **supersedes the mechanism** of its
  stage 2 and answers its open question Q-1. See "Relationship" below.
- [`work/pluggable-storage/`](../pluggable-storage/) — the other axis. Storage is where the bytes
  live; anchoring is where the hash is committed. Conflating the two already caused the bug fixed
  in #256.

## The model

```rust
trait AnchorWriter {
    async fn anchor(&self, content_hash: &str, locator: &str) -> Result<AnchorRecord>;
}

enum Anchor {
    Solana(String),                  // memo signature
    Ethereum { tx: String, contract: String },
    Arweave(String),                 // the item's own inclusion is the anchor
    None,
}
```

`core/src/storage/sqlite.rs` carries an `Anchor` rather than a Solana-only `solana_tx` column.
The existing SPL Memo path becomes `SolanaAnchor`. Legacy rows read as `Anchor::Solana(..)`.

## Relationship to chain-agnostic, which changes that plan

`work/chain-agnostic/` stage 2 says "stop writing the memo", and its **Q-1** asks whether that
should ship behind an environment flag so it can be re-enabled without a redeploy.

**This folder is the better answer to Q-1.** An `AnchorWriter` selection is the flag, expressed
as a type rather than a boolean: choosing `Anchor::None` *is* "stop writing the memo", and
choosing `SolanaAnchor` restores it, with no dead code path and no second mechanism to reason
about.

So the sequence changes: implement pluggability **first**, then turning Solana off is a
configuration change rather than a code deletion. That is strictly safer, because it is
reversible on a deploy rather than on a revert.

What does **not** change: the gate. `work/chain-agnostic/` D-1 still holds — enumeration must be
measured working before any deployment selects `Anchor::None`, because the Solana memo is
currently the only thing that can list an owner's anchored memories. Pluggability makes the
switch reversible; it does not make it safe to flip early.

## The insight #69 already recorded

From `work/a2a-bridge/backlog.md`: for an ERC-8004-routed attestation, the
`validationResponse` transaction **is** the anchor. The on-chain `responseHash` already commits
the data, so that path gains anchoring for free rather than adding a second one.

That is the strongest argument for this trait over deletion: a future protocol integration may
*supply* an anchor, and there needs to be somewhere to put it.

## Scope

- `core/src/anchor/` — the trait, the `Anchor` enum, and `AnchorRecord`.
- `core/src/solana/` — wrap the existing memo path as `SolanaAnchor`. No behaviour change.
- `core/src/storage/sqlite.rs` — an additive, idempotent migration carrying `Anchor`. Legacy
  rows become `Anchor::Solana(..)`. Follow the `PRAGMA table_info` pattern of
  `migrate_visibility_column`, and keep `solana_tx` readable for ever.
- `EthereumAnchor` — the type and a stub only. No Ethereum client in this step; that is #75.
- Config — anchor selection, defaulting to today's behaviour so an unchanged deployment behaves
  identically.

## Out of scope

- An actual Ethereum client. Issue #75.
- Removing `core/src/solana/mod.rs`. Solana becomes optional, not absent; every memo **reader**
  stays for ever per `work/chain-agnostic/` D-2.
- Storage backends. That is `work/pluggable-storage/`.
- The `Signer` seam. Anchoring is about where a hash is published, not about who signs.
