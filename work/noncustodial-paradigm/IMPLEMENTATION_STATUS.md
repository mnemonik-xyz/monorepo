# Non-Custodial Paradigm — Implementation Status & Handoff

> Pickup ledger for continuing the build in a fresh session. Design spec is
> [`design.md`](./design.md) (22 sections, 12 diagrams); this file tracks
> *implementation* progress, the verification recipe, gotchas, and exact next
> steps.

## Where things are
- **Repo / branch:** `mnemonik-dev/monorepo`, branch
  **`claude/mnemonic-noncustodial-paradigm`** (renamed from
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
| 6 | _this commit_ | Encrypted shared memories core: `core/src/encrypt.rs` X25519 ECIES seal-to-N-recipients (`seal_to_recipients`/`open`/`public_from_secret`, serde envelope) — public/private/shared-to-N, recipient-only decryption (8 unit tests). Native-only for now. |

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
```
Expect ~212 lib + ~207 + many integration tests, **0 failures**.

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

## Remaining integration (depends on external/chain infra — not code-only)

The deterministic, code-only core of every wave is landed + tested. What's left
needs live infrastructure (a chain/validator, a deployed contract, or a verified
wasm build) that can't be exercised in a code-only session, so it was
deliberately not stubbed:

- **W4 — on-chain allowance (design §6, Level 2).** A user-controlled escrow
  contract the operator draws from per signed receipt. Needs the contract
  deployed on Arc/Solana; x402 already provides the non-custodial per-call rail,
  so this is an additive convenience for non-interactive clients. Mirror the
  x402 verifier shape: a server-side allowance-receipt verifier + draw path.
- **W5 — anchor + expose.** (a) Anchor each per-owner `merkle::commitment_root`
  on Solana per epoch (operator-relayed, mirrors the existing SPL-memo write —
  needs a validator to test). (b) Return inclusion proofs in the
  `mnemonic_recall` tool output + a client-side proof check against the anchored
  root. (c) Opt-in f32 precision tier stored *inside* the signed artifact
  (public memories only) — a `codec`/schema + recall change.
- **W6 — wire + expose.** (a) Map the Ed25519 identity → its X25519 recipient
  key (standard birational conversion, §18) in the client key-management layer.
  (b) Per-type visibility defaults at sign time (deliverable→shared,
  feedback→public, validation-evidence→shared) + store the `SealedEnvelope` on
  Arweave for shared/private writes. (c) Flip `encrypt` on for wasm once the
  in-browser RNG wiring is verified against the `cross-lang-build` gate (so the
  webapp can decrypt client-side).

## Standing rule
Every wave: full §21 gate, find-callers-before-delete, and never push a wave that
leaves another module red (see design §21).
