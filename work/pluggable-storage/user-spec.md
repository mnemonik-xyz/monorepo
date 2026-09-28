# User Spec: Pluggable Storage

## What changes

Today, every anchored memory goes to Arweave via Irys. That is not configurable —
the backend is baked in. This feature makes the storage backend a named, swappable
choice so the protocol is not permanently coupled to a single network.

Arweave stays the default and the only production-ready option on day one. A stub
for Filecoin is added to prove the seam compiles and to reserve the extension point.

## What users and operators see

**Operators** gain a new config key, `storage_backend`. Today it is implicitly
`arweave`. After this feature it is explicit and checked at startup:

```toml
storage_backend = "arweave"   # the only eligible value today
```

Trying to start with `storage_backend = "s3"` or any backend not on the eligible
list causes an immediate error with a plain-language explanation, before any
network calls are made.

**Owners restoring their memories** see no change. The restore command reads
locators out of the database and fetches bytes from wherever the locator says they
are. If every locator says `ar://`, every fetch goes to Arweave, same as today.

**The database gains no user-visible columns.** The internal `arweave_tx` column
is renamed conceptually to `locator`, and existing bare Arweave transaction IDs are
silently treated as `ar://<txid>`. No data is lost, no manual migration is required,
and the export format continues to show the raw locator string.

## Eligibility — the gate that matters

Not every storage system is eligible for anchored mode. Arweave is the default
precisely because it meets a bar that most systems do not:

| Requirement | Plain meaning |
|---|---|
| Durability without operator action | Once stored, the data survives without the operator renewing a subscription or keeping a server running |
| Content addressing or an immutable reference | A locator must refer to exactly one version of the bytes forever |
| Third-party readable without the operator | A verifier must be able to fetch the artifact without asking us |
| Enumerable by owner | Restore must be able to list every item an identity ever stored |

Arweave via Irys meets all four today. Filecoin with a paid pinning contract may
meet the first two conditionally; its stub is present to hold the interface but is
not production-eligible until those conditions are documented and tested.

**S3, R2, and operator-managed disk are never eligible for anchored mode.** They
depend on the operator to stay available, which is exactly the custodial risk that
`arweave-as-source-of-truth` removed. They are acceptable as a cache layer but
never as the sole copy.

## What is NOT in scope

- Running two backends at once or mirroring one memory to two places.
- Moving existing Arweave memories to a new backend (Arweave data is permanent and
  cannot be deleted or moved, so "migration" here would be copying, not moving).
- Changes to anchoring (which chain receives the hash commitment). That is the
  separate `chain-agnostic` feature on the same decoupling sequence.
- Any S3 or operator-disk path for anchored mode, ever.
