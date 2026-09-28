# Tech spec: pluggable-anchoring

**Feature:** pluggable-anchoring
**Issue:** #70
**Depends on:** nothing (this is Phase 3α, a prerequisite for ERC-8004)

---

## 1. Module: `core/src/anchor/`

Create a new module with three items:

### 1.1 `AnchorWriter` trait

```rust
// core/src/anchor/mod.rs

use anyhow::Result;

/// The result of a successful anchor write (or a no-op).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AnchorRecord {
    /// Discriminated anchor type tag, e.g. "solana", "ethereum", "none".
    pub anchor_type: String,
    /// Chain-specific reference: Solana tx signature, Ethereum tx hash, etc.
    /// Empty string for Anchor::None.
    pub anchor_ref: String,
}

/// Enum of all supported anchor variants. Stored as JSON in the `anchor_json`
/// column (see §3). Legacy rows that carry only a `solana_tx` are read back as
/// `Anchor::Solana(solana_tx)`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "ref")]
pub enum Anchor {
    Solana(String),
    Ethereum { tx: String, contract: String },
    None,
}

impl Anchor {
    pub fn to_record(&self) -> AnchorRecord {
        match self {
            Anchor::Solana(sig) => AnchorRecord {
                anchor_type: "solana".into(),
                anchor_ref: sig.clone(),
            },
            Anchor::Ethereum { tx, .. } => AnchorRecord {
                anchor_type: "ethereum".into(),
                anchor_ref: tx.clone(),
            },
            Anchor::None => AnchorRecord {
                anchor_type: "none".into(),
                anchor_ref: String::new(),
            },
        }
    }

    /// Construct a legacy `Anchor::Solana` from the raw `solana_tx` column
    /// value when `anchor_json` is absent (NULL or empty). An empty string
    /// maps to `Anchor::None` (local-mode rows).
    pub fn from_legacy_solana_tx(solana_tx: &str) -> Self {
        if solana_tx.is_empty() || solana_tx.starts_with("local:") {
            Anchor::None
        } else {
            Anchor::Solana(solana_tx.to_string())
        }
    }
}

/// Async anchor-write abstraction. Each backend implements this.
#[async_trait::async_trait]
pub trait AnchorWriter: Send + Sync {
    /// Commit `content_hash` to the backing chain (or do nothing for None).
    /// `locator` is the Arweave tx id of the COSE envelope — included in
    /// the Solana SPL Memo payload so the two can be correlated on-chain.
    async fn anchor(&self, content_hash: &str, locator: &str) -> Result<AnchorRecord>;
}
```

### 1.2 File layout

```
core/src/anchor/
    mod.rs          — AnchorWriter trait, Anchor enum, AnchorRecord
    solana.rs       — SolanaAnchor (wraps existing SolanaClient::write_memo)
    ethereum.rs     — EthereumAnchor (stub)
    none.rs         — NoneAnchor (no-op)
    factory.rs      — build_anchor_writer(config) -> Arc<dyn AnchorWriter>
```

---

## 2. `SolanaAnchor` — wrapping the existing SPL Memo path

`core/src/anchor/solana.rs` wraps `SolanaClient::write_memo`. No behaviour change.

```rust
// core/src/anchor/solana.rs

use anyhow::Result;
use std::sync::Arc;
use crate::solana::SolanaClient;
use super::{AnchorRecord, AnchorWriter};

pub struct SolanaAnchor {
    client: Arc<SolanaClient>,
    keypair: solana_sdk::signature::Keypair,
}

impl SolanaAnchor {
    pub fn new(client: Arc<SolanaClient>, keypair: solana_sdk::signature::Keypair) -> Self {
        Self { client, keypair }
    }
}

#[async_trait::async_trait]
impl AnchorWriter for SolanaAnchor {
    async fn anchor(&self, content_hash: &str, locator: &str) -> Result<AnchorRecord> {
        let memo = serde_json::json!({"h": content_hash, "a": locator, "v": 3}).to_string();
        let sig = self.client.write_memo(&self.keypair, &memo).await?;
        Ok(AnchorRecord {
            anchor_type: "solana".into(),
            anchor_ref: sig,
        })
    }
}
```

The existing `SolanaClient` in `core/src/solana/mod.rs` is unchanged. `submit_memo`, `confirm_tx`, `list_memo_anchors`, and `read_memo` all stay. Memo readers are permanent (D-2).

---

## 3. SQLite migration — additive and idempotent

### 3.1 New columns

Add two nullable columns to `attestations`:

| Column | Type | Default | Purpose |
|---|---|---|---|
| `anchor_type` | `TEXT` | `NULL` | Discriminant: `"solana"`, `"ethereum"`, `"none"`. NULL means legacy row. |
| `anchor_ref` | `TEXT` | `NULL` | Chain-specific reference string. Empty string for `Anchor::None`. |

Alternatively, a single `anchor_json TEXT` column holding the `Anchor` enum serialised to JSON can replace both. The two-column layout is preferred: it is SQL-queryable without JSON functions and mirrors the existing `write_mode`/`visibility` pattern.

### 3.2 Migration function

Follow the `migrate_visibility_column` pattern: gate each `ALTER TABLE` on `attestations_has_column`, wrap in `BEGIN IMMEDIATE` / `COMMIT`, log a context string on error.

```rust
fn migrate_anchor_columns(conn: &Connection) -> anyhow::Result<()> {
    let need_type = !attestations_has_column(conn, "anchor_type")?;
    let need_ref  = !attestations_has_column(conn, "anchor_ref")?;

    conn.execute_batch("BEGIN IMMEDIATE;")
        .context("opening anchor_columns migration transaction")?;

    let do_migration = || -> anyhow::Result<()> {
        if need_type {
            conn.execute("ALTER TABLE attestations ADD COLUMN anchor_type TEXT", [])
                .context("adding attestations.anchor_type")?;
        }
        if need_ref {
            conn.execute("ALTER TABLE attestations ADD COLUMN anchor_ref TEXT", [])
                .context("adding attestations.anchor_ref")?;
        }
        // Backfill: any row with a real solana_tx and no anchor_type yet is a
        // legacy Solana-anchored row. Fill it to avoid redundant per-read
        // inference. local: synthetic ids and empty strings stay NULL (they map
        // to Anchor::None in the reader but need no column value).
        conn.execute(
            "UPDATE attestations
                SET anchor_type = 'solana', anchor_ref = solana_tx
              WHERE anchor_type IS NULL
                AND solana_tx <> ''
                AND solana_tx NOT LIKE 'local:%'",
            [],
        )
        .context("backfilling anchor_type/anchor_ref for legacy Solana rows")?;
        Ok(())
    };

    match do_migration() {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .context("committing anchor_columns migration")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}
```

Call `migrate_anchor_columns(&conn)?;` in both `SqliteStore::open` and `SqliteStore::in_memory`, after the existing migration chain (last in sequence).

### 3.3 Reading legacy rows

When `anchor_type` is NULL, construct the `Anchor` at read time:

```rust
Anchor::from_legacy_solana_tx(&row.solana_tx)
```

This covers both the backfill-missed edge case and any DB opened before this migration runs.

### 3.4 Writing new rows

`save_attestation` receives an `AnchorRecord`. It writes both `anchor_type` and `anchor_ref` alongside the existing `solana_tx` (kept for compatibility — its value for a `SolanaAnchor` row is the same signature; for `Anchor::None` it is an empty string, which is the existing local-mode convention).

---

## 4. Config

### 4.1 Config key

```toml
# config.toml (or env ANCHOR_TYPE)
anchor_type = "solana"   # "solana" | "none" | "ethereum"
```

Default: `"solana"`. An operator who does not set this key gets the existing behaviour unchanged.

### 4.2 Factory

`core/src/anchor/factory.rs`:

```rust
pub fn build_anchor_writer(
    anchor_type: &str,
    solana_client: Option<Arc<SolanaClient>>,
    solana_keypair: Option<Keypair>,
) -> anyhow::Result<Arc<dyn AnchorWriter>> {
    match anchor_type {
        "solana" => {
            let client = solana_client.context("SOLANA_RPC_URL required for anchor_type=solana")?;
            let kp = solana_keypair.context("SOLANA_KEYPAIR required for anchor_type=solana")?;
            Ok(Arc::new(SolanaAnchor::new(client, kp)))
        }
        "none" => Ok(Arc::new(NoneAnchor)),
        "ethereum" => Ok(Arc::new(EthereumAnchor::stub())),
        other => anyhow::bail!("unknown anchor_type: {other}"),
    }
}
```

---

## 5. `EthereumAnchor` — type and stub only

```rust
// core/src/anchor/ethereum.rs

use anyhow::Result;
use super::{AnchorRecord, AnchorWriter};

/// Stub implementation. Returns an empty `anchor_ref`. No Ethereum client.
/// A real implementation ships in issue #75.
pub struct EthereumAnchor;

impl EthereumAnchor {
    pub fn stub() -> Self { EthereumAnchor }
}

#[async_trait::async_trait]
impl AnchorWriter for EthereumAnchor {
    async fn anchor(&self, _content_hash: &str, _locator: &str) -> Result<AnchorRecord> {
        Ok(AnchorRecord {
            anchor_type: "ethereum".into(),
            anchor_ref: String::new(),
        })
    }
}
```

---

## 6. Tasks

**T1 — `core/src/anchor/` module skeleton**
Create `mod.rs` with `AnchorWriter` trait, `Anchor` enum, `AnchorRecord` struct, and `Anchor::from_legacy_solana_tx`. Add `none.rs` (`NoneAnchor`) and `ethereum.rs` (`EthereumAnchor` stub). Wire the new module into `core/src/lib.rs`. Unit-test `Anchor::from_legacy_solana_tx` for empty string, `local:` prefix, and a real base58 signature.

**T2 — `SolanaAnchor` implementation**
Create `core/src/anchor/solana.rs` wrapping `SolanaClient::write_memo`. Add `factory.rs` with `build_anchor_writer`. No change to existing code paths at this point. Unit-test the factory for all three `anchor_type` values.

**T3 — SQLite migration**
Add `migrate_anchor_columns` to `core/src/storage/sqlite.rs`. Call it in `open` and `in_memory` after the existing chain. Update `save_attestation` to accept and persist an `AnchorRecord` in the two new columns alongside the existing `solana_tx`. Write a migration round-trip test: insert a legacy row (no `anchor_type`), reopen the DB, verify `Anchor::from_legacy_solana_tx` returns the right variant.

**T4 — Wire anchor selection into the write path**
Update the server's startup code to call `build_anchor_writer` with the config value. Plumb the resulting `Arc<dyn AnchorWriter>` through to the attestation write path, replacing the direct `SolanaClient::write_memo` call. Verify that `anchor_type = "solana"` is a no-op change end-to-end (existing tests pass unchanged).

**T5 — Config parsing and integration tests**
Add `anchor_type` to the config struct with default `"solana"`. Add an integration test that boots the server with `anchor_type = "none"`, writes a memory, and asserts that `solana_tx` is empty and `anchor_type = "none"` is stored in the DB.

**T6 — Docs and decisions update**
Update `work/pluggable-anchoring/decisions.md` with any decisions made during implementation. Update `work/chain-agnostic/` to note that Q-1 is answered by this feature.

---

## 7. Decisions

### D-1 — Default stays `"solana"`

An operator who does not set `anchor_type` gets the existing behaviour. This ensures zero unintended behaviour change on upgrade.

### D-2 — Keep all memo readers

`SolanaClient::list_memo_anchors`, `read_memo`, and `get_tx_signers` are not removed or gated. Historical rows with real Solana signatures must remain recoverable forever, regardless of what the current `anchor_type` is. Reader code is permanent; only the writer is swapped.

### D-3 — EthereumAnchor is stub only in this issue

No Ethereum client, no web3 dependency, no on-chain call. The stub compiles, the config parser accepts it, and the DB schema supports it. The real implementation is issue #75. Shipping the stub here means #75 will not require any breaking change to the trait, the config key, or the DB schema.
