# Tech Spec: Pluggable Storage

## 1. BlobStore trait

New file: `core/src/blobstore/mod.rs`

```rust
/// A content-addressed or immutable-reference blob store eligible for
/// `anchored` mode.
///
/// Implementations are responsible for:
///   - persisting bytes so they survive without operator action
///   - producing a `Locator` that is stable and globally resolvable
///   - enumerating every locator ever stored for a given `owner` tag
///
/// The `owner` parameter on `put` and `list` is an opaque tag (the owner's
/// pubkey string) used by the backend's own index. It is not a permission
/// check; it is the label the backend records so `list` can enumerate
/// per-identity.
#[async_trait::async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, bytes: &[u8], owner: &str) -> anyhow::Result<Locator>;
    async fn get(&self, locator: &Locator) -> anyhow::Result<Vec<u8>>;
    async fn list(&self, owner: &str) -> anyhow::Result<Vec<Locator>>;
}
```

The trait is object-safe. All callers that need to hold a backend use
`Arc<dyn BlobStore>`. The `Send + Sync` bounds are required because the arc
crosses async boundaries.

## 2. Locator type

New file: `core/src/blobstore/locator.rs`

```rust
/// A self-describing, backend-agnostic reference to one immutable blob.
///
/// String form: `<scheme>://<id>`
///   - `ar://<base64url-txid>`   — Arweave transaction id
///   - `ipfs://<cid>`            — IPFS CID (reserved; no implementation yet)
///
/// Legacy bare Arweave ids (no scheme prefix) parse as `ar://`. This handles
/// all existing rows in `attestations.arweave_tx` without a data migration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locator {
    pub scheme: String,
    pub id: String,
}

impl Locator {
    pub fn arweave(txid: impl Into<String>) -> Self { ... }

    /// Parse `"ar://<id>"`, `"ipfs://<id>"`, or a bare id (treated as ar://).
    pub fn parse(s: &str) -> anyhow::Result<Self> { ... }

    /// Return the raw id portion without the scheme.
    pub fn id(&self) -> &str { &self.id }

    /// True for bare Arweave ids and `ar://` locators.
    pub fn is_arweave(&self) -> bool { self.scheme == "ar" }
}

impl std::fmt::Display for Locator {
    /// Always emits `<scheme>://<id>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { ... }
}

impl std::str::FromStr for Locator {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> { Locator::parse(s) }
}
```

Parsing rules:
- If the string contains `://`, split on first `://`, left side is scheme, right
  is id.
- If the string contains no `://`, treat the whole string as a bare Arweave txid
  and parse as `ar://<string>`. This preserves backward compatibility with every
  existing row in the database.
- Unknown schemes parse successfully (round-trip fidelity) but `BlobStore`
  dispatch returns `Err` at runtime.

## 3. ArweaveClient implements BlobStore

`core/src/arweave/mod.rs` gains an `impl BlobStore for ArweaveClient` block.

Mapping:
- `put(bytes, owner)` → `write_item(data, keypair, extra_tags)` where
  `extra_tags` includes `("Owner", owner)`. Returns `Locator::arweave(txid)`.
  The keypair is required for signing; see design ambiguity note D-A1 below.
- `get(locator)` → assert `locator.is_arweave()`, then `read(locator.id())`.
  Returns an error if the locator is not an Arweave locator.
- `list(owner)` → delegates to `GraphQlClient::list_anchored` filtered by the
  `Owner` tag. Returns `Vec<Locator>` with `ar://` scheme.

The existing `write`, `write_bytes`, `write_item`, and `read` methods are kept
unchanged because they are called from several sites that are not yet converted
to the trait.

## 4. SQLite migration: arweave_tx becomes locator

`core/src/storage/sqlite.rs` gains a new idempotent migration function:
`migrate_arweave_tx_to_locator`.

Migration steps (additive, wrapped in `BEGIN IMMEDIATE`):

1. Add column `attestations.locator TEXT` (nullable; `ALTER TABLE ... ADD COLUMN`).
2. Backfill: for every row where `locator IS NULL`:
   - If `arweave_tx` starts with a known scheme prefix (`ar://`, `ipfs://`), copy
     it verbatim into `locator`.
   - Otherwise, set `locator = 'ar://' || arweave_tx`.
3. Add index: `CREATE INDEX IF NOT EXISTS idx_attestations_locator ON
   attestations(locator)`.

The existing `arweave_tx` column is NOT dropped or renamed. It continues to be
read and written by all existing code paths until a follow-up task migrates every
callsite to use `locator`. This migration therefore only prepopulates `locator`
for all existing rows and ensures new rows written during the transition can be
read via either column.

`migrate_arweave_tx_to_locator` is added to the sequence in both `open()` and
`in_memory()`, after `migrate_plaintext_on_arweave_column`.

### is_anchored_arweave_tx compatibility

`is_anchored_arweave_tx(s: &str)` is updated to also return `true` for strings
starting with `ar://` that are followed by a non-empty id. The function is used
by several query predicates; it must handle both old and new locator formats
during the transition.

## 5. restore/mod.rs uses &dyn BlobStore

`core/src/restore/mod.rs` `fetch_restorable` currently takes `gateway:
&ArweaveClient`. It becomes:

```rust
pub async fn fetch_restorable(
    store: &dyn BlobStore,
    enumerated: &[(String, Option<String>)],
) -> (Vec<RestorableItem>, Vec<(String, String)>)
```

Inside the function, the bare `arweave_tx` string from `enumerated` is parsed
into a `Locator` before calling `store.get(&locator)`. The `RestorableItem.arweave_tx`
field keeps its name for now; the type is updated to `Locator` in a follow-up.

All callers of `fetch_restorable` (currently only the MCP restore tool in
`mcp/src/tools.rs`) are updated to pass an `Arc<dyn BlobStore>` dereferenced
as `&*arc`.

The `enumerate_anchored` function is unchanged: it returns `(String, Option<String>)`
tuples. Parsing those strings into `Locator`s is `fetch_restorable`'s job.

## 6. Config: storage_backend

Config struct (location: `core/src/config.rs` or equivalent) gains:

```rust
pub struct Config {
    // ... existing fields ...
    /// Blob storage backend for anchored mode.
    /// Valid values: "arweave", "filecoin" (stub only; not production-eligible).
    /// Default: "arweave".
    #[serde(default = "default_storage_backend")]
    pub storage_backend: String,
}

fn default_storage_backend() -> String { "arweave".to_string() }
```

At startup, before any network connections are opened, the backend is validated
against the eligibility list:

```rust
pub fn check_storage_backend(backend: &str) -> anyhow::Result<()> {
    match backend {
        "arweave" => Ok(()),
        "filecoin" => anyhow::bail!(
            "storage_backend = \"filecoin\" is a stub and not eligible for anchored mode. \
             Use \"arweave\"."
        ),
        other => anyhow::bail!(
            "Unknown storage_backend {:?}. Valid values: \"arweave\".",
            other
        ),
    }
}
```

This function is called in `main` (or wherever the server initialises its
services) before `ArweaveClient::new` is called. A bad config is a startup
failure, not a runtime error.

## 7. FilecoinStore stub

New file: `core/src/blobstore/filecoin.rs`

```rust
/// Stub implementation of `BlobStore` for Filecoin.
///
/// Not production-eligible: no HTTP client, no deal-making logic.
/// Exists to prove the `BlobStore` seam compiles and to reserve the
/// extension point. All methods return `Err`.
pub struct FilecoinStore;

#[async_trait::async_trait]
impl BlobStore for FilecoinStore {
    async fn put(&self, _bytes: &[u8], _owner: &str) -> anyhow::Result<Locator> {
        anyhow::bail!("FilecoinStore is a stub and not implemented")
    }
    async fn get(&self, _locator: &Locator) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("FilecoinStore is a stub and not implemented")
    }
    async fn list(&self, _owner: &str) -> anyhow::Result<Vec<Locator>> {
        anyhow::bail!("FilecoinStore is a stub and not implemented")
    }
}
```

No feature flag is needed. The type is always compiled in. The startup check
ensures it cannot be activated.

## 8. Tasks

**T1 — BlobStore trait and Locator type**
Create `core/src/blobstore/mod.rs` and `core/src/blobstore/locator.rs`. Implement
`Locator::parse` (with legacy bare-id fallback), `Display`, and `FromStr`. Unit
tests: round-trip `ar://`, `ipfs://`, and bare ids.

**T2 — ArweaveClient implements BlobStore**
Add `impl BlobStore for ArweaveClient` in `core/src/arweave/mod.rs`. Update
`put` to emit `Locator::arweave(txid)`. Update `get` to dispatch on scheme.
Update `list` to parse GraphQL results into `Vec<Locator>`. Unit tests: mock HTTP
server, assert locator scheme and id round-trip.

**T3 — FilecoinStore stub and config check**
Create `core/src/blobstore/filecoin.rs`. Add `storage_backend` to config and
`check_storage_backend`. Call the check at startup. Unit tests: valid config
passes, `filecoin` returns a clear error, unknown values return a clear error.

**T4 — SQLite locator migration**
Add `migrate_arweave_tx_to_locator` to `core/src/storage/sqlite.rs`. Register it
in `open()` and `in_memory()`. Update `is_anchored_arweave_tx` to accept `ar://`
prefixed strings. Unit tests: bare id → `ar://` backfill, prefixed id kept as-is,
idempotent on second open.

**T5 — restore/mod.rs uses &dyn BlobStore**
Change `fetch_restorable`'s `gateway` parameter from `&ArweaveClient` to
`&dyn BlobStore`. Parse bare arweave_tx strings to `Locator` inside
`fetch_restorable`. Update callers. Tests: inject a `MockBlobStore` that records
calls and returns canned bytes, assert `RestoreReport`.

**T6 — Integration smoke test**
In the existing `in_memory` restore test (if one exists) or a new test, wire
`ArweaveClient` through `BlobStore`, run `fetch_restorable`, and confirm the
report is correct. Confirms the full stack compiles end-to-end with the trait in
place.

## 9. Decisions

**D-1 Arweave default unchanged.**
The only production-eligible backend on day one is Arweave via Irys. No behaviour
change for any existing deployment.

**D-2 Locator is self-describing.**
`attestations.locator` stores `ar://<txid>` (or `ipfs://<cid>` etc.) rather than
a bare id. Legacy bare ids in `arweave_tx` are normalised to `ar://` by the
migration so restore always parses a locator without out-of-band scheme knowledge.

**D-3 S3 is never eligible for anchored mode.**
Operator-controlled durability is the custodial risk that `arweave-as-source-of-
truth` removed. S3, R2, and operator disk may be used as read caches but never as
the sole copy of an anchored memory. The config check enforces this at startup.

## Design ambiguities

**D-A1 — BlobStore::put and the Solana keypair.**
`ArweaveClient::write_item` requires a `&Keypair` to sign the ANS-104 data item
for Irys. The `BlobStore::put` signature above does not include the keypair because
other backends (Filecoin, IPFS pinning) do not sign with a Solana key. Options:

1. Keep `put(bytes, owner)` as-is and have `ArweaveClient` hold the keypair
   internally (set at construction time, same as `base_url`). This is the
   simplest option and the one implied by the README design.
2. Add a `put_signed(bytes, owner, keypair)` method to `BlobStore` and default-impl
   it to return an error ("not a signing backend"), relying on callers to use the
   right variant.

Option 1 is recommended. The keypair is already tied to the operator's node
identity; injecting it at `ArweaveClient::new` is consistent with how
`base_url` and `network` are already constructor parameters. Option 2 leaks
Arweave-specific concepts into the trait.

Resolution needed before T2.

**D-A2 — Owner tag on Arweave items.**
`BlobStore::list(owner)` requires the backend to enumerate items by owner. For
Arweave via Irys this is a GraphQL tag filter. The current `write_item` does not
consistently apply an `Owner` tag; it applies `App-Name` and
`Content-Type`. Adding `("Owner", owner)` to every new upload is the prerequisite
for `list` to work. Existing items without the tag are reachable via Solana memo
history (the existing `enumerate_anchored` path), so the gap is only for new
uploads. The spec assumes this tag will be added in T2. Confirm the Irys GraphQL
indexer actually indexes custom tags for ANS-104 bundle items (see the enumeration
caveat in the README — Irys bundles were invisible to gateway GraphQL at one
point).

Resolution needed before T2.
