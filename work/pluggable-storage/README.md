# pluggable-storage — parking lot

Status: **TODO — brief, then run user-spec-planning.**

Siblings, and the reason this is its own folder:

- [`work/chain-agnostic/`](../chain-agnostic/) — removes the **anchor** (where the hash is
  committed). This folder is about **storage** (where the bytes live). Two different axes that
  are easy to conflate, which is why they are not one feature.
- [`work/arweave-as-source-of-truth/`](../arweave-as-source-of-truth/) — shipped. Made Arweave
  the only copy of an anchored memory. That is what makes a second backend meaningful: the
  backend is now the sole home of the data, not a mirror of the operator's database.
- [`work/sealed-memories/`](../sealed-memories/) — orthogonal. Sealing is about what the bytes
  are; this is about where they go.

## The model

One trait, several backends:

```rust
trait BlobStore {
    async fn put(&self, bytes: &[u8], owner: &str) -> Result<Locator>;
    async fn get(&self, locator: &Locator) -> Result<Vec<u8>>;
    async fn list(&self, owner: &str) -> Result<Vec<Locator>>;
}
```

`ArweaveClient` becomes one implementation. Same shape as the `AnchorWriter` trait that issue
#70 asks for, on the other axis.

## Why this is cheaper than it looks

**Verification does not depend on storage.** `codec::sign::verify_artifact(cose_bytes)` works on
bytes whatever produced them: the COSE_Sign1 envelope plus the blake3 content hash are the
integrity mechanism. A second backend therefore touches no verification code, no signing code
and no schema of the artifact itself.

`core/src/rebuild.rs` is likewise backend-agnostic already — it takes bytes, not a URL.

## Why it is more expensive than it looks

Two things, both learned the hard way in `work/arweave-as-source-of-truth/`:

**1. The locator must be self-describing.** Today `attestations.arweave_tx` is a bare string, so
a row does not say which backend holds it. It becomes `ar://<txid>`, `ipfs://<cid>` and so on,
or a `{backend, ref}` pair. This is the one real schema change, and it must be additive:
existing bare ids are implicitly `ar://`, and must keep resolving.

**2. Enumeration is per backend and is the hard part.** Verified 2026-09-27: our Arweave items
are invisible to Arweave-schema gateways by tag, because Irys bundles them — see
`work/chain-agnostic/` F6. Every backend needs its own answer to "list everything this owner
stored", and a backend that cannot answer it cannot support restore. A backend that stores bytes
but cannot enumerate them is a write-only hole.

## Eligibility bar — the decision that matters

Backends are **not** interchangeable, and the spec must say so rather than presenting a menu.
A backend is eligible for `anchored` mode only if it provides all four:

| Requirement | Why |
|---|---|
| Durability without operator action | Anything the operator must keep paying for or pinning re-creates the custody the protocol removes |
| Content addressing or an immutable id | A mutable pointer breaks the hash commitment |
| Third-party read without the operator | A verifier must not need us |
| Enumeration by owner | Otherwise restore cannot work |

Assessed against that bar:

- **Arweave (via Irys)** — eligible. Pay once, permanent. Current default.
- **Filecoin / IPFS with a paid pinning service** — conditionally eligible. Content-addressed and
  third-party readable, but durability depends on the pin surviving. Needs an explicit statement
  of who pays and what happens when they stop.
- **Walrus, or another pay-once network** — plausibly eligible; needs the same four checks.
- **S3, R2, or an operator disk** — **NOT eligible for `anchored`.** Operator-controlled
  durability is exactly the custodial position `work/arweave-as-source-of-truth/` removed. Fine
  as a cache. Never as the only copy.

Sequencing note: adding an ineligible backend would quietly undo shipped work, which is why the
bar comes before the trait.

## Scope

- `core/src/blobstore/` — the trait, `Locator` parsing and display, and the eligibility doc.
- `core/src/arweave/` — implement the trait; keep the concrete client for the Irys specifics.
- `core/src/storage/sqlite.rs` — locator migration, additive, with bare ids reading as `ar://`.
- `core/src/restore/` — enumerate through the trait instead of the concrete client.
- Config — backend selection, and a refusal to start when an ineligible backend is set for
  `anchored`.

## Out of scope

- Multi-backend writes, or mirroring one memory to two backends. Attractive, and a separate
  decision about cost.
- Migrating existing memories between backends. Arweave data cannot be moved or deleted.
- Anchors. That is `work/chain-agnostic/`.
