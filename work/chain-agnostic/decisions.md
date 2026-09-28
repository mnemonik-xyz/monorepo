# Decisions log: chain-agnostic

Append-only. Owner decisions, task reports and audit findings go here.

---

## Fixed inputs (owner)

- **2026-09-28.** Make Mnemonic chain-agnostic. First implementation step: remove Solana
  anchoring.
- **2026-09-28.** Document how deep the Solana integration goes before planning the removal.

## Audit findings (2026-09-28)

- **F1. The coupling is wide but shallow.** `solana_sdk` appears 41 times across 25 files, but
  34 of those uses are `Keypair`, `Signer`, `Pubkey` or `Signature` — Ed25519 key handling with
  no network access. Solana is acting as a crypto library in most of the codebase.
- **F2. Real chain code sits in three places.** `core/src/solana/mod.rs` (499 lines: RPC, SPL
  Memo, transaction assembly); `mcp/src/payment.rs:696 verify_usdc_transfer` plus one
  `getTransaction` call; and the Irys upload endpoint `uploader.irys.xyz/tx/solana`, which
  accepts an ANS-104 item signed by the Solana key.
- **F3. Payments are already dual-rail.** `EvmPaymentConfig` and `EVM_RPC_URL` exist beside the
  Solana USDC path, so payment is not a blocker for removing the anchor.
- **F4. Two surfaces carry Solana shapes without calling the chain.** 66 `did:sol:` references,
  and the `solana_tx TEXT NOT NULL` column with its queries.
- **F5. The upload is Solana-signed.** `uploader.irys.xyz/tx/solana` and
  `solana_pubkey_to_arweave_address` both derive from the same key, so the Solana keypair is
  load-bearing for putting bytes on Arweave at all — a deeper dependency than the memo.
- **F6. The memo is the only working enumeration source.** Carried from
  `work/arweave-as-source-of-truth/` D-5, verified 2026-09-27: Arweave-schema gateways return
  zero items for our `App-Name` tag; the Irys endpoint returns a full page for the same filter.
  Our items are Irys-bundled, so Arweave gateways index the bundle, not the items.

---

## Decisions

### D-1. Order: enumeration first, memo removal second (2026-09-28)

Not a preference. Removing the memo writer before enumeration works elsewhere makes every
existing anchored memory unenumerable, and therefore unrestorable, while its bytes remain intact
on Arweave. Nothing else in this plan can destroy value; this can.

The switch is gated on a **measured** result — the Irys enumeration returning a superset of the
memo enumeration for a real wallet — because the failure mode is silent. An empty index and a
rejected query look identical to the caller, which is how a misconfigured endpoint survived
unnoticed until 2026-09-27.

### D-2. Remove the memo writer; keep every memo reader for ever (2026-09-28)

`read_memo`, `list_memo_anchors` and `parse_anchor_memo` stay, as does the `solana_tx` column.
A memo cannot be created retroactively for an item already anchored, so deleting the readers
would strand every memory written before this change.

### D-3. Identity is last, and needs its own spec (2026-09-28)

Swapping `solana_sdk::Keypair` for `ed25519-dalek` is mechanical, but the COSE `kid` must keep
its base58 spelling (`core/src/codec/sign.rs:61`, read back at `:121`). Changing it stops every
existing attestation from verifying — including bytes already permanent on Arweave, which cannot
be rewritten. Unrecoverable failures get their own spec and their own golden-vector proof.

### D-4. Removing the memo reduces redundancy, and we say so (2026-09-28)

After this change, Irys is the only index that can see our items. That is a genuine loss of
redundancy traded for a per-write fee and a confirmation round-trip. It is a reasonable trade,
but it is a trade, and the documents must not present it as pure gain.

## Open questions

- **Q-1. ANSWERED 2026-09-28 — see `work/pluggable-anchoring/`.** Neither. An environment flag
  and a straight removal were the wrong two options. Anchor pluggability (#70) expresses "no
  Solana anchor" as a configuration value — `Anchor::None` — so there is no second mechanism
  and no dead code path. Selecting it is reversible on a deploy; deleting the write path would
  be reversible only on a revert. Pluggability therefore lands BEFORE stage 2, which becomes a
  configuration change. The D-1 gate is unaffected: reversible is not the same as safe, and a
  measured enumeration result is still required.
- **Q-2.** Is the single-provider dependency on Irys acceptable, or should a second index be
  found before the memo is switched off? This is the substantive question in D-4, and it is the
  owner's to answer.
- **Q-3.** Does `verify_usdc_transfer` stay on Solana indefinitely, or does the EVM rail become
  the default once ERC-8004 work lands?

## Stage 1 shipped (2026-09-28)

`GatewayFlavour` (Arweave | Irys), dual-query schema, millisecond timestamp 
conversion, and client-side ordering are all live on main in 
`core/src/arweave/graphql.rs`. Shipped as part of #246–#252
(`work/arweave-as-source-of-truth/`).

Tests added:
- `arweave_query_contains_sort_and_block` — Arweave schema integrity
- `irys_query_omits_sort_and_block` — Irys schema correctness  
- `timestamp_units_irys_converts_ms_to_s` — ms→s conversion
- `timestamp_units_arweave_passes_through` — Arweave unchanged
- `irys_results_are_sorted_oldest_first` — client-side ordering
- `paginates_until_last_page` — pagination
- `flavour_detection_from_url` — URL-based detection

**Gate status:** Pending live verification — the Irys enumeration returning a
superset of the memo enumeration for a real production wallet must be confirmed
before stage 2 starts. This requires running against real data with
`GRAPHQL_URL=https://uploader.irys.xyz/graphql`.

**Stage 2 is blocked until the gate passes.**
