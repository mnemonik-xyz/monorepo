# Tech spec — recover traction stats from chain

## Problem

The server migration lost `attestations.db`. The webapp traction strip
(`/stats`) and Analytics page (`/analytics/attestations`) read SQLite only,
so all lifetime numbers reset to zero. The anchored history survives on
Arweave/Solana; the user chose the GraphQL approach (chain = source of
truth, no bulk re-import into the DB).

## Design

### Enumeration (core)

`core/src/arweave/graphql.rs` — `GraphQlClient::list_anchored`: paginated
gateway GraphQL query (`first: 100`, cursor loop, `HEIGHT_ASC`) filtered by
`App-Name: mnemonic-protocol` and optionally `owners`. Operators configure
base58 Solana pubkeys; `solana_pubkey_to_arweave_address` derives the
gateway's owner form (`base64url_nopad(sha256(pubkey))` — the arbundles
`ownerToAddress` rule for Ed25519 data items).

`core/src/arweave/recovery.rs` — `snapshot_chain`: for items without a
`Producer` tag (all legacy uploads), fetch the payload once and decode
COSE_Sign1 → canonical CBOR → `producer`. Structural decode only, no
signature verification (counts don't need it; anyone can verify
independently). Unreadable payload ⇒ producer `None`, still counted.

### Merge (mcp)

`mcp/src/chain_stats.rs` — `ChainStatsCache` (tokio `RwLock` snapshot,
refreshed on startup then every `CHAIN_STATS_REFRESH_SECS`; a failed
refresh keeps the previous snapshot) and the pure `merge_stats`:

- **saved_onchain** = |chain tx ids ∪ DB participate rows' `arweave_tx`|.
  Union covers both directions: historic items absent from the fresh DB,
  and brand-new writes the gateway hasn't indexed yet.
- **unique_users** = |normalized chain producers (`did:sol:` stripped)
  ∪ DB non-empty `owner_pubkey`|.
- **saved_on_node** = saved_onchain + DB local-only rows.
- **buckets**: chain block-day for recovered items, exact DB day when the
  row exists locally (DB wins); pending-block items count in totals only.

`SqliteStore::recovery_facts()` feeds the DB side (arweave_tx, owner, day,
write_mode — aggregates only, no content).

### Handlers

`public_stats_handler` and `analytics_attestations_handler` use the merged
path when a snapshot exists, else the unchanged DB-only path. Lock
discipline: the async snapshot read happens before the sqlite mutex.
Webapp wire shapes are untouched — no frontend changes.

### Forward fix

`sign_memory` (inline) and `sign_callback_handler` (deferred) now upload
via `write_item` with `Producer` + `Created-At` tags — both values are
already public inside the payload; the tags just make them queryable so
future recovery needs zero payload fetches.

## Config (all optional; feature off when wallets empty)

| Env | Default | Meaning |
| --- | --- | --- |
| `CHAIN_STATS_WALLETS` | `` | comma-separated base58 pubkeys (public only) |
| `CHAIN_STATS_GRAPHQL_URL` | `https://arweave.net/graphql` | gateway GraphQL |
| `CHAIN_STATS_GATEWAY_URL` | `https://arweave.net` | payload fetches |
| `CHAIN_STATS_REFRESH_SECS` | `3600` | snapshot refresh (min 60) |

## Testing

- core: gateway pagination, owner-filter inlining, GraphQL error surfacing,
  address derivation, COSE producer extraction, legacy backfill (httpmock).
- mcp: merge semantics — chain-only recovery, union dedup, local rows,
  pending-block items (pure unit tests).
- Live verification against the real wallet address: pending (operator is
  locating the address; only the public key is needed).

## Known limits

- Local-mode rows and local-only users from the lost DB are unrecoverable.
- Stdio-path uploads have `producer = server DID` → one distinct user.
- Recall/Ledger content is NOT rebuilt by this feature (would need a
  re-import + re-embed pass — deliberately out of scope per user's
  "no double data" preference).
