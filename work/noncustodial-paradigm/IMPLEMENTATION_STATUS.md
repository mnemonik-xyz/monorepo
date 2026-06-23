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

## Next waves — concrete starting points
- **Wave 2 — programmatic client-signing.** Promote the deferred sign path
  (`tools.rs:854` `awaiting_signature` → `api.rs:128` `sign_callback_handler`
  verifies COSE `kid == signer_pubkey` → persists `signer = owner = user`) so the
  SDK/CLI sign the canonical CBOR locally instead of only via the browser
  `approve_url`. Make client-sign the default. ~80% exists; this is wiring +
  exposing the prepare→sign→submit handoff to non-browser clients.
- **Wave 3 — remove operator signing. ⚠ LANDMINE:** `seed.rs:330+` RAG seeding
  hard-codes the inline operator-sign path (`sign_artifact` @ `tools.rs:1057`
  with `state.keypair`). Removing operator signing breaks startup seeding unless
  seeding gets its own server-identity sign path OR is explicitly exempted
  (operator signing its *own* knowledge base ≠ a user authoring a memory).
  Also check `download-knowledge` / `chat.rs` consumers.
- **Wave 4 — remove API keys + allowance.** Delete `mnm_` keys, `balance` mode,
  `/api-keys`, `/deposit`, `credit_deposit`, `deduct_balance` (payment.rs +
  main.rs routes + mcp.rs gate). Schema tables `api_keys`/`payment_events`/
  `x402_nonces` are in `core/src/storage/sqlite.rs` — don't break migrations or
  unrelated reads. Add the on-chain allowance draw path. This is the biggest
  blast-radius wave — do it in a clean session with the feature-gated tests.
- **Wave 5 — verifiable recall / drop SQL-as-truth.** Arweave canonical for
  content + Solana-anchored per-owner Merkle commitments; recall returns inclusion
  proofs; demote the vector store to a rebuildable cache. Precision tiers decided
  (§16): default compressed, opt-in f32 in the signed artifact (public memories
  only).
- **Wave 6 — encrypted shared memories.** X25519 key-wrapping to `{client,
  provider, evaluator}` for confidential deliverables; recall client-side
  post-decrypt. Public is the current default.

## Standing rule
Every wave: full §21 gate, find-callers-before-delete, and never push a wave that
leaves another module red (see design §21).
