# Non-Custodial Paradigm — Implementation Status & Handoff

> Pickup ledger for continuing the build in a fresh session. Design spec is
> [`design.md`](./design.md) (22 sections, 12 diagrams); this file tracks
> *implementation* progress, the verification recipe, gotchas, and exact next
> steps.

## Start here (fresh agent)
1. `git checkout claude/mnemonic-noncustodial-paradigm` (HEAD = `0f6b0df`).
2. `apt-get install -y libdbus-1-dev pkg-config` (keyring dep — else the build
   panics in `libdbus-sys`).
3. Run the §21 gate below to confirm green, then read **Remaining work** and
   pick the top open item. Every wave's deterministic core is done; what's left
   is either an external-infra integration or the f32 *writer* wiring.
4. One pre-existing env-only test failure is expected here:
   `core::identity::ensure_rolls_back_on_partial_failure` fails *only when run as
   root* (root bypasses the `0o555` perm the test relies on). It passes in CI's
   non-root runner. Not a regression — ignore it.

## Where things are
- **Repo / branch:** `mnemonik-dev/monorepo`, branch
  **`claude/mnemonic-noncustodial-paradigm`**, HEAD **`0f6b0df`** (renamed from
  `claude/arco-agent-capabilities-5irqsg`; the old remote branch still exists with
  the same history up to the docs).
- **Arco app** lives in a *separate* repo `mnemonik-dev/Arco-Agent` (branch
  `claude/arco-agent-capabilities-5irqsg`) — the integration + a pointer doc
  (`docs/MNEMONIC_NONCUSTODIAL_PARADIGM.md`).

## Done (verified + pushed)
| Wave | Commit | What |
|---|---|---|
| 0 / F1 | `3ac84b8` | Gate `/admin/stats` P&L behind `ADMIN_TOKEN` (fail-closed). `admin_authorized()` in `main.rs`; `admin_token` on `Config` + `McpState`. |
| 1 / p1 | `91a1a60` | EVM USDC verifier primitive in `payment.rs`: `EvmPaymentConfig`, `verify_evm_usdc_transfer`, pure `match_erc20_transfer` + 3 unit tests. |
| 1 / p2 | `78c7865` | Wire EVM x402 into the live gate: network-aware `check_x402`/`check_payment` (route by proof `network`), 402 advertises EVM option, `EVM_RPC_URL`/`EVM_USDC_TOKEN`/`EVM_TREASURY` → `Option<EvmPaymentConfig>` on `McpState`. |
| 2 | `dfe7b9c` | Programmatic (browser-free) client-signing: `sign_memory` deferred response returns the unsigned canonical CBOR inline (`canonical_cbor_b64`) + `client_sign` submit contract + `content_hash`, so SDK/CLI/agent clients COSE-sign locally and POST `/api/sign-callback` with no browser. New `test_programmatic_client_sign_without_pending_get`. |
| 3 | `4a66636` | Remove operator signing for remote users: the operator key may inline-sign ONLY its own memory (`owner == operator`); every JWT write owned by a different identity (incl. explicit `mode:"local"`) routes to client-signing. `write_mode` plumbed through the pending bundle → callback persists the real mode. Defense-in-depth guard in `sign_memory_inline`. seed.rs self-authored exemption documented. |
| 4 | `528bd75` | Remove custodial API keys + balance ledger (clean break §8): deleted `create_api_key`/`get_balance`/`deduct_balance`/`credit_deposit`/`refund_balance`/`check_balance`/`extract_api_key`/`record_refund_failed` + `/api-keys`,`/balance`,`/deposit` routes; `PaymentGate::Proceed` is now a unit variant; `PAYMENT_MODE ∈ {none,x402}`; DoS quota re-keyed on `blake3(x402 tx_sig)`. Schema tables retained for migration safety. |
| 5 | `d9df6f1` | Verifiable-recall core: `core/src/merkle.rs` per-owner Merkle commitment (set semantics, RFC-6962 domain separation, odd-node promotion) with `commitment_root`/`prove`/`verify` (9 unit tests); `SqliteStore::owner_content_hashes` supplies the leaf set from the rebuildable cache; `integration_merkle_recall.rs` proves end-to-end inclusion + cross-owner non-forgeability. |
| 6 | `fe77e9f` | Encrypted shared memories core: `core/src/encrypt.rs` X25519 ECIES seal-to-N-recipients (`seal_to_recipients`/`open`/`public_from_secret`, serde envelope) — public/private/shared-to-N, recipient-only decryption (8 unit tests). Native-only for now. |
| 5b | `4c044cc` | `mnemonic_recall` now returns `merkle_commitment` = `{root, proofs{content_hash→[steps]}}` computed from the rebuildable cache (authenticated owner only; `null` for the anonymous pool). `tools::build_merkle_commitment` + 3 integration tests (`recall_merkle_commitment.rs`) proving each returned proof verifies against the root via `merkle::verify`, tamper rejection, and the anonymous-pool null case. |
| 5c | `daa1a32` | Rebuild-index-from-stored-artifacts: `core/src/rebuild.rs` `rebuild_row`/`rebuild_rows` reconstruct recall-index rows from signed COSE bytes alone (COSE-verify → decode CBOR → recover content/owner/tags/content_hash + decompress embedding). Makes "SQLite = rebuildable cache" real, not aspirational. `integration_rebuild.rs` (4 tests): lossless field round-trip, recall over a rebuilt store matches the original top-1, the embedding is provably lossy (the gap the f32 tier closes), tampered artifact rejected. |
| 5d | `0f6b0df` | f32 precision tier (READ side): artifacts may carry the full f32 embedding in `metadata.embedding_f32` (base64 LE); `rebuild_row` prefers it (lossless, `Precision::F32`) and falls back to the compressed copy (`Precision::Compressed`). `f32_embedding_to_bytes`/`from_bytes` codec shared by producer+consumer. `integration_rebuild.rs` (+3 → 7): exact f32 rebuild, f32-preferred-when-both, compressed-only tagged correctly. |

**Wave 0 F2/F3/F4 intentionally skipped** — they harden the custodial API-key
machinery that Wave 4 deletes (throwaway). F1 was the only Wave-0 item that
survives the paradigm.

## Verification recipe (the §21 gate — run before every commit)
```bash
cargo fmt --all
cargo build -p mnemonic-mcp
cargo clippy --workspace --all-targets -- -D warnings
# IMPORTANT: integration tests need the test-support feature, else you get
# bogus "unresolved import mnemonic_mcp::test_support" / 0-test runs.
cargo test -p mnemonic-mcp --features test-support
cargo test -p mnemonic-core            # merkle (9) + encrypt (8) + rebuild (7) + …
```
CI runs clippy twice: `--workspace --lib --bins` (no feature) AND
`--workspace --all-targets --features mnemonic-mcp/test-support`. Both must be
clean. Expect **mcp 567 / 0**; core all green except the root-only env failure
noted in *Start here*.

## Gotchas already paid for (don't rediscover)
1. **`test-support` feature is mandatory for tests.** `cargo test` *without* it
   fails to compile some integration tests (`mnemonic_mcp::test_support`). CI uses
   the feature.
2. **`McpState` has 9 construction sites.** Any new field must be added to ALL:
   `main.rs` (real), `mcp.rs` (cfg-test), `chat.rs`, `test_support.rs` (×4),
   and `mcp/tests/{pending_expiry,sign_callback,pending_authz}.rs`. The last three
   + the lib-test one only compile under `--features test-support`, so plain
   clippy won't catch them — **run the feature-gated tests**. Mechanical insert:
   `perl -i -pe 's{^(\s*)(<prev_field>,)$}{$1$2\n$1<new_field>: <val>,}' <files>`.
3. **Commit-signing server flaps with 503.** `git commit` may fail
   "signing server returned status 503"; just retry (loop up to ~5×).
4. **`McpState` payment fields** are also mirrored on `Config` (`config.rs`) and
   built in `main.rs` — keep the three in sync.

## Running a local `mnemonic-mcp` (for live round-trips)
- Needs system **`libdbus-1-dev`** (`apt-get install -y libdbus-1-dev`) — the
  keyring dep, else `libdbus-sys` build panics.
- Build with the embedder: `cargo build -p mnemonic-mcp --release --features local-embed`.
- The fastembed model is fetched from HuggingFace LFS (blocked here); mirror it:
  download `Qdrant/all-MiniLM-L6-v2-onnx` files (`model.onnx` + tokenizer*.json +
  config.json + special_tokens_map.json) via `https://hf-mirror.com/...` into an
  hf-hub cache dir and run with `FASTEMBED_CACHE_DIR=<dir> HF_HUB_OFFLINE=1`.
- Required env: `MCP_JWT_SECRET` (32-byte base64), `MCP_REFRESH_SALT` (32-byte
  base64), `STORAGE_MODE=local PAYMENT_MODE=none EMBED_PROVIDER=fastembed`.
- Tool calls (`sign`/`verify`) require a **Bearer JWT** (HS256, `iss=mcp.mnemonik.xyz`,
  `aud=mcp`, signed with the base64-DECODED `MCP_JWT_SECRET`); `mnemonic_recall` is
  allowlist-open. (`mnemonik-noncustodial`: scripts to mint/run exist in the
  prior session's scratch but are not committed.)

## Remaining work

### A. Code-only and testable now (no external infra) — do these first
- **f32 tier — WRITER side (mcp).** The read side is done (W5d). Wire
  `mnemonic_sign_memory` to emit `metadata.embedding_f32` on an opt-in,
  **public-only** write (e.g. a `precision: "f32"` request field; reject f32 on
  non-public visibility — embeddings leak content via inversion, §16/§17). The
  artifact build lives in `tools.rs` (`sign_memory_inline` + `sign_memory_deferred`,
  both build the metadata block). Reference producer:
  `core/tests/integration_rebuild.rs::make_artifact_f32`. Test via the existing
  harness: sign with `precision:f32` → rebuild → `Precision::F32` + exact embedding.
- **Recall: surface `precision` + cross-check.** Optionally tag recall results
  with the precision tier and (when an anchored root is configured) include the
  rebuild check. Small, additive.

### B. On-chain Merkle-root anchoring — DECISION NEEDED, then build
Recall already returns `{root, proofs}` (W5b); the missing half is publishing a
**trusted** root a client can check those proofs against. The choice is along
three axes — pick one per axis:

- **Where / mechanism**
  1. **Solana SPL Memo** *(recommended v1)* — reuse `solana::SolanaClient::write_memo`
     (already used for content anchors). Memo body `{v, owner_pubkey, root, epoch, count}`.
     Cheap (~5000 lamports), zero new infra, operator-relayed. Limitation: a memo
     is a *log*, not state — "latest root for owner X" needs an indexer or
     `getSignaturesForAddress` on a per-owner/operator anchor account + newest-wins.
  2. **Solana PDA program** *(Level 2)* — an Anchor program with a per-owner PDA
     holding the latest `(root, epoch)`. O(1) canonical read; costs rent + a
     program deploy.
  3. **EVM registry contract (Arc/Base)** *(Level 2, best for Arco/ERC-8004)* —
     `mapping(ownerKey => (root, epoch))` + an on-chain `verifyInclusion(...)`.
     Lets *another contract* trust a memory. Aligns with the EVM x402 rail (Wave 1);
     needs deploy + gas + a mapping from the Ed25519 owner id to the registry key.
  4. **Arweave-tagged** — root as an Arweave tx tagged by owner+epoch. Same
     durability as content but a weak *independent* witness (same operator uploads
     it) — not recommended as the sole anchor.
- **Granularity / cadence**: per-write (always current, most txs) vs **per-epoch
  batched** (design §16 default — amortized cost, but a freshness window where a
  just-written memory isn't yet covered) vs per-owner (what we built) vs a global
  Merkle-of-owner-roots (one tx total/epoch, but proofs need an extra layer).
- **Who anchors / binding**: **operator-relayed** (matches everything else; the
  operator can delay/withhold → that's the censorship surface, acceptable since
  the operator is replaceable) vs **user self-anchors** (no operator dependency,
  but the user transacts each epoch) vs **hybrid** (operator by default, user can
  self-anchor as fallback — mirrors the §7 censorship-resistance line). **Strong
  option:** anchor a **user-signed** root (COSE over `{owner, root, epoch}`) and
  have the operator merely *relay* it — then even the operator can't forge an
  owner's root, fully consistent with the self-sovereign paradigm.

  **Recommended starting point:** Solana SPL Memo · per-epoch · operator-relayed,
  carrying a user-signed `{owner, root, epoch}`. Smallest delta, reuses
  `write_memo`, no program deploy; graduate to a PDA/EVM registry when O(1) reads
  or on-chain proof verification (Arco) are needed. **All of this needs a live
  validator to test end-to-end** — hence it was not stubbed code-only.

### C. Other external-infra integrations (not code-only)
- **W4 — on-chain allowance (design §6, Level 2).** User-controlled escrow the
  operator draws from per signed receipt. Needs the contract deployed; x402
  already covers per-call non-custodial payment. Mirror the x402 verifier shape.
- **W6 — wire + expose.** (a) Map Ed25519 identity → X25519 recipient key
  (birational conversion, §18) in the client key-management layer. (b) Per-type
  visibility defaults at sign time (deliverable→shared, feedback→public,
  validation-evidence→shared) + store the `SealedEnvelope` on Arweave for
  shared/private writes. (c) Flip `core::encrypt` on for wasm once the in-browser
  RNG wiring is verified against the `cross-lang-build` gate (webapp decrypt).

## Module map (what landed in `core/`)
- `merkle.rs` — per-owner set commitment + inclusion proofs (`commitment_root`,
  `prove`, `verify`, `parse_hex32`/`to_hex32`). Pure, all targets.
- `rebuild.rs` — `rebuild_row`/`rebuild_rows` + `Precision` + f32 codec. Native-only.
- `encrypt.rs` — X25519 ECIES `seal_to_recipients`/`open`. Native-only.
- `storage::SqliteStore::owner_content_hashes` — leaf set for the commitment.
- mcp `tools::build_merkle_commitment` — attaches `{root, proofs}` to recall.

## Standing rule
Every wave: full §21 gate (both clippy invocations), find-callers-before-delete,
and never push a wave that leaves another module red (see design §21).
