---
created: 2026-09-28
status: draft
size: L
branch: feat/drop-solana-anchor
---

# Tech Spec: chain-agnostic Mnemonic, starting with the Solana anchor

> **Order:** this is one step of a larger sequence. See
> [`work/DECOUPLING-SEQUENCE.md`](../DECOUPLING-SEQUENCE.md) for where it sits and what must
> land first.


## How deep the coupling goes

Measured on 2026-09-28, not estimated. The result changes the plan, so it comes first.

`solana_sdk` appears **41 times across 25 files**. That number overstates the problem:

| Imported item | Count | What it really is |
|---|---|---|
| `signature::{Keypair, Signer}` | 22 | Ed25519 key handling |
| `signature::Keypair` | 9 | Ed25519 key handling |
| `pubkey::Pubkey` | 5 | a 32-byte key with base58 display |
| `signature::Signature` | 1 | a 64-byte Ed25519 signature |
| other | 4 | — |

So **34 of 41 uses treat `solana_sdk` as a crypto library, not as a chain.** Those files never
reach the network. Replacing them is a dependency swap, not a decoupling.

Real chain code is confined to three places:

1. **`core/src/solana/mod.rs`** — 499 lines. JSON-RPC, the SPL Memo program id, transaction
   assembly, `write_memo` / `submit_memo` / `read_memo` / `list_memo_anchors` / `confirm_tx`.
2. **`mcp/src/payment.rs:696` `verify_usdc_transfer`** plus one `getTransaction` RPC call.
3. **The previous bundler upload**, `https://{previous-bundler-host}/tx/solana`, which accepts an ANS-104 data
   item signed by the Solana key. `solana_pubkey_to_arweave_address` derives the owner address
   from the same key.

Two surfaces carry Solana shapes without calling the chain: **66 `did:sol:` references**, and
the `solana_tx TEXT NOT NULL` column with the queries over it.

**Conclusion.** The anchor is the shallowest dependency and the only one that costs money per
write. It is the right first target. Identity is the deepest and has the worst failure mode, so
it is last.

## The blocking prerequisite

**The Solana memo is currently the only working enumeration source.** Verified on 2026-09-27
(`work/arweave-as-source-of-truth/decisions.md` D-5): Arweave-schema gateways
(`arweave-search.goldsky.com`, `permagate.io`) return **zero** items for
`App-Name: mnemonic-protocol`, while the previous bundler GraphQL endpoint returns a full page of 100 for
the same filter. Our items are bundled by the previous bundler, so Arweave gateways index the containing bundle
rather than the items.

Stop writing memos before fixing enumeration and every anchored memory becomes unenumerable,
and therefore unrestorable, while its bytes sit safely on Arweave. That is the one way this
work can destroy value, so it is stated as an ordering constraint rather than advice.

## Solution

Four stages. Only stage 1 and 2 are specified in detail; the rest are scoped so the sequence is
visible and the first step does not foreclose them.

### Stage 1 — enumerate from an index that sees our items

Point the GraphQL source at the previous bundler and make the query match its schema. The previous bundler rejects two fields
the Arweave schema accepts, and asking for either fails the whole request:

```text
Unknown argument "sort" on field "Query.transactions"
Cannot query field "block" on type "Transaction"
```

So: no `sort` argument, and the time is on the node as `timestamp` in **milliseconds**, not
under `block` in seconds.

Do not delete the Arweave-schema query. Both must exist, because the endpoint is configurable
and an operator may point at either. Introduce a `GatewayFlavour` (`Arweave` | `Bundler`) that
selects the query text and the timestamp parsing, and derive it from the configured URL with an
explicit override.

Ordering moves client-side for the previous bundler flavour, since the gateway cannot sort.

**Acceptance, and it is a gate rather than a checklist item:** for a wallet with existing
anchored memories, the previous bundler enumeration returns a superset of the memo enumeration. Until that
holds, stage 2 does not start.

### Stage 2 — stop writing the memo

- `WriteMode::Anchored` uploads to Arweave and does not call Solana.
- `solana_tx` stays in the schema and keeps its value for existing rows. New rows store the
  empty string, which `RowFact` already documents as a valid state.
- `read_memo`, `list_memo_anchors` and `parse_anchor_memo` all stay. Legacy rows must remain
  enumerable and verifiable for ever.
- The delivery check drops its Solana comparison. It already re-fetches from Arweave and
  COSE-verifies, which is the stronger half; the memo comparison becomes optional.
- `mnemonic_verify` keeps accepting a `solana_tx` lookup for legacy rows.

The operator's Solana wallet still pays the previous bundler for uploads. This stage removes the memo, not the
chain.

### Stage 3 — payments (scoped, not specified)

`verify_usdc_transfer` stays. An EVM rail already exists next to it (`EvmPaymentConfig`,
`EVM_RPC_URL`, x402), so payments are already dual-rail. Nothing here blocks stages 1 and 2.

### Stage 4 — identity (scoped, not specified)

Swapping `solana_sdk::Keypair` for `ed25519-dalek` is mechanical — same curve, same bytes — but
touches every signing and verification path.

**The hard constraint: the COSE `kid` must stay the base58 spelling.** It is set at
`core/src/codec/sign.rs:61` and read back at `:121`. Change it and **every existing attestation
stops verifying**, including bytes already permanent on Arweave. That is unrecoverable, so this
stage needs its own spec, its own golden-vector proof, and is last.

`did:sol:` is 66 references and lives in production data. Renaming it is a separate decision
with no technical benefit.

## Decisions

### Decision 1 — remove the memo writer, keep every memo reader

Asymmetric on purpose. Writing is a cost we choose per new memory; reading is how existing
memories stay reachable. Deleting the reader would strand every memory written before this
change, and there is no way to re-create a memo for an item already anchored.

### Decision 2 — the enumeration switch is a gate, not a step

Stage 2 is blocked on a measured superset result from stage 1, on real data. A code review
cannot establish it, because the failure is silent: an empty index and a failed query are
indistinguishable to the caller, which is exactly how the previous misconfiguration survived.

### Decision 3 — support both gateway schemas rather than switching

The endpoint is configurable, so a single hardcoded schema is wrong for somebody. Selecting the
query by flavour also keeps the Arweave path alive for the day Arweave gateways do index
bundled items.

### Decision 4 — `solana_tx` keeps its column and its `NOT NULL`

Changing a `NOT NULL` column needs a table rebuild in SQLite, and the empty string is already
an accepted value there. The column also stays meaningful for legacy rows for ever, so removing
it is never correct.

## Testing

- **Enumeration parity.** Against a wallet with known anchored items, assert the previous bundler
  enumeration is a superset of the memo enumeration. This is the gate for stage 2.
- **Schema exactness per flavour.** Assert the previous bundler query contains no `sort` and no `block`, and
  that the Arweave query still contains both. A single test asserting one shape would silently
  break the other.
- **Timestamp units.** Previous-bundler milliseconds convert to seconds; Arweave seconds pass through. A
  missing divide is wrong by a factor of a thousand and nothing else validates the magnitude.
- **Anchored write makes no Solana call.** Point the RPC at a URL that fails on contact and
  assert an anchored write still succeeds.
- **Legacy rows stay verifiable.** A row with a real `solana_tx` still verifies, and
  `mnemonic_verify` still resolves it.
- **Restore covers both.** One memory with a memo and one without both restore in a single run.

## Risks

- **Enumeration gap, the only way to lose data.** Mitigated by making stage 1 a measured gate
  rather than a review item.
- **Previous-bundler schema drift.** Our items are visible only through the previous bundler, so an upstream schema change
  breaks enumeration silently. Mitigation: the live `#[ignore]` test, and never adding a field
  the schema does not define.
- **Single-provider dependency.** Removing the memo leaves the previous bundler as the only index that sees our
  items. That is a real reduction in redundancy, traded for cost. Say it plainly rather than
  presenting the change as pure gain.
- **A future reader assumes "no Solana" means the readers went too.** Mitigation: Decision 1 is
  stated at the call sites, not only here.
