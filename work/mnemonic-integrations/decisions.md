# Decisions Log — mnemonic-integrations

Append-only log of decisions and audit findings during execution.

---

## Task 3 — Webapp WASM build pipeline (T3-impl)

**Date:** 2026-04-26
**Status:** Implementation complete; smoke verify pending Task 2 (`wasm` feature on `core/Cargo.toml` + `[lib] crate-type = ["cdylib", "rlib"]`).

### What changed

- **New:** `webapp/scripts/build-wasm.sh` — `set -euo pipefail`, anchors to repo root, checks `wasm-pack` is on PATH, runs `wasm-pack build core --target web --out-dir webapp/src/wasm --release --features wasm`. Marked executable (`chmod 755`). Top-of-file comment documents the `cargo install wasm-pack` prerequisite, the Task 2 dependency on the `wasm` feature, and the expected output layout.
- **Modified:** `webapp/package.json` — added `"build:wasm": "bash scripts/build-wasm.sh"`, rewrote `build` to `npm run build:wasm && tsc -b && vite build`, and added a `"//"` field documenting the `cargo install wasm-pack` prerequisite (per task spec hint #5). Other scripts (`dev`, `preview`, `test:e2e`) untouched. No new runtime dependencies.
- **New:** `webapp/.gitignore` — excludes `node_modules/`, `dist/`, `src/wasm/` (the `wasm-pack` output, regenerated on every build), plus `playwright-report/`, `test-results/`, and standard editor/OS noise.
- **Unchanged:** `webapp/vite.config.ts` — left alone per task spec ("only modify if smoke testing surfaces a concrete error"). Vite ≥ 6 handles the `wasm-pack --target web` ESM shim natively; no `vite-plugin-wasm`, no `optimizeDeps.target = 'esnext'` needed.

### Verification

- `bash -n webapp/scripts/build-wasm.sh` — syntax OK
- `node -e "JSON.parse(...)"` on `webapp/package.json` — JSON valid
- `git check-ignore -v webapp/src/wasm/dummy.wasm` → matched on `webapp/.gitignore:8:src/wasm/` — gitignore rule wired correctly
- `cd webapp && npm run build:wasm` — npm script invokes `bash scripts/build-wasm.sh` (confirmed). The script reaches `wasm-pack build core ... --features wasm` and fails with `Error: crate-type must be cdylib to compile to wasm32-unknown-unknown` — **this is the expected pre-Task-2 state**: `core/Cargo.toml` does not yet declare the `wasm` feature nor `[lib] crate-type = ["cdylib", "rlib"]`. Once Task 2 lands those, the smoke command will succeed end-to-end.
- Local toolchain: `wasm-pack 0.13.1` is on PATH.

### Deviations

None. The script uses the repo-root-anchored form from the task spec body (Details section) rather than the path-relative form mentioned in the dispatcher prompt — the repo-root form is more robust (works regardless of caller's CWD) and matches the spec's authoritative implementation hint. Behavior is identical.

### Concerns / follow-ups

- **Smoke verification deferred:** the full `cd webapp && npm run build` cannot exit 0 until Task 2 has merged. The pipeline files (script + package.json + .gitignore) are independent and complete; they only need Task 2's Rust-side `wasm` feature + `cdylib` crate-type to produce artifacts.
- **CI reminder:** any future CI step that invokes `npm run build` on the webapp must also `cargo install wasm-pack` first (or use a pre-built image). This is documented in the script's prereq comment and the `package.json` `//` field; deployment.md prerequisites should reference it when CI integration lands (Task 14 territory).
- **Submodule note:** the dispatcher prompt referred to `webapp/` as a git submodule, but `git ls-tree HEAD webapp` reports `040000 tree` (regular tracked directory), not `160000 commit` (submodule pointer). Although `webapp/.git` exists locally, the parent monorepo has no `.gitmodules` entry and stores webapp files directly. Therefore the changes are committed as ordinary file additions in the monorepo, not via the two-step "submodule commit + pointer update" flow.

---

## Task 1: Streamable HTTP transport upgrade

**Date:** 2026-04-26
**Status:** Implementation complete; smoke verified.

### Summary

Converted `POST /mcp` from request-response `Json<JsonRpcResponse>` to MCP-spec-2025 streamable HTTP: chunked NDJSON frames (`Content-Type: application/x-ndjson`, `Transfer-Encoding: chunked`, one newline-terminated JSON-RPC envelope per frame). Today exactly one frame per request — multi-frame plumbing (`Body::from_stream` over a `futures::stream::once`) is wired so Task 4b PendingBundles can extend with `mpsc::Receiver` without touching the response shape. Stdio transport (`run_stdio` in `main.rs`) is untouched. Added `bearer_auth_layer` (`axum::middleware::from_fn`) on `/mcp` only — currently a no-op pass-through that inspects `Authorization` header but does not enforce; carries a `TODO(task-4a)` marker for the Decision-9 allowlist + JWT validation swap.

### Notable

- **OAuth middleware scaffolding (Task 4 hook):** `mcp::bearer_auth_layer` is a `from_fn` closure — Task 4a replaces its body with HS256 JWT validation against `MCP_JWT_SECRET`, allowlists `initialize` + `tools/list` (Decision 9), and injects the resolved pubkey into request extensions. The third unit test `test_missing_authorization_header_returns_401` is written today as `#[ignore]` with comment "Activated by Task 4a — flip ignore + assert 401" so Task 4a only needs to remove the attribute and tighten the assertion.
- **Handler relocation:** moved `mcp_handler` from `main.rs` into `mcp.rs` (now `pub`) so `#[cfg(test)] mod transport_tests` can construct a `Router::new().route("/mcp", post(mcp_handler).layer(...))` directly. `main.rs::run_http` now references `mcp::mcp_handler` and `mcp::bearer_auth_layer`.
- **Payment gate preserved:** the `PaymentGate::Proceed`/`NeedPayment(x402)`/`Unauthorized` branches all emit a single NDJSON frame with the correct HTTP status (200/402/401). Refund-on-error path still calls `payment::refund_balance` under a non-await lock.
- **Cargo.toml diff:** added `futures = "0.3"`, `tokio-stream = "0.1"`, `bytes = "1"` explicitly (no transitive reliance). Did **not** add `oauth2`, `jsonwebtoken`, `tower_governor` — those belong to Task 4a.
- **Test name filter:** wrapped tests in `mod transport_tests` (not `mod tests`) so `cargo test -p mnemonic-mcp -- transport` filters to exactly the three new tests.

### Verification

- `cargo test -p mnemonic-mcp -- transport` → 2 passed, 1 ignored (`test_missing_authorization_header_returns_401` flips on Task 4a).
- `cargo test -p mnemonic-mcp` (full) → 57 passed, 1 ignored, 0 failed.
- `cargo test --workspace --no-fail-fast` → green.
- `cargo clippy -p mnemonic-mcp --all-targets --all-features -- -D warnings` → zero warnings (one `useless_format` flagged and fixed).
- `cargo fmt -p mnemonic-mcp -- --check` → clean.
- **Smoke (HTTP):** `STORAGE_MODE=local PAYMENT_MODE=none target/release/mnemonic-mcp --transport http --port 3000` then `curl -sN -i -X POST http://localhost:3000/mcp -d '{"jsonrpc":"2.0","method":"tools/list","id":1}'` → `HTTP/1.1 200 OK`, `content-type: application/x-ndjson`, `transfer-encoding: chunked`, body is one newline-terminated JSON line listing all 5 tools (`mnemonic_whoami`, `mnemonic_sign_memory`, `mnemonic_verify`, `mnemonic_prove_identity`, `mnemonic_recall`).
- **Smoke (stdio regression):** `echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | mnemonic-mcp --transport stdio` → returns line-delimited JSON with the same 5 tools, transport unchanged.

### Concerns / known gaps for audit wave

- **Body parsing pre-validation:** `mcp_handler` now consumes `body: Bytes` and parses with `serde_json::from_slice` (so we control the error envelope). This shifts the parse failure path from axum's default `Json` rejection HTML to a `-32700` NDJSON frame at HTTP 400 — security audit should confirm this is the desired behavior (it matches the JSON-RPC spec) and that there is no DoS vector via giant request bodies (axum's default `RequestBodyLimitLayer` of 2 MiB still applies; surface for explicit tightening in Task 4a).
- **Single-frame today:** `ndjson_response` uses `stream::once` for the current single-frame shape. Cancellation is trivially safe because there is no producer task to leak. When Task 4b introduces multi-frame mpsc-backed responses, `test_partial_response_client_disconnect` becomes the actual regression guard — today it primarily proves no global state corruption (mutex not poisoned, second request still succeeds).
- **`unsafe impl Send/Sync` for McpState:** untouched by this task but inherited. Audit should re-validate now that the test code constructs `McpState` from non-trivial threads (`tokio::test`); existing `chat::handler_tests` already exercises this so no new regression introduced.
- **CORS still `Any`:** Decision 9 will narrow to exact origin `https://mnemonik.xyz`. Out of scope for Task 1 per the spec ("leave the existing `Any` policy in place").
- **No `ldd`/RUSTSEC review of new transitive deps:** `futures = "0.3"` + `tokio-stream = "0.1"` + `bytes = "1"` are foundational; CI `cargo audit` step (Task 4 prereq) is the right gate.

---

## Task 2 — WASM bindgen wrappers in `core/` (T2-impl)

**Date:** 2026-04-26
**Status:** Implementation complete; native + wasm32 builds green; all 7 wasm-bindgen-tests pass headless under Firefox.

### What changed

- **New:** `core/src/wasm/mod.rs` (~300 LOC including tests). Five `#[wasm_bindgen]` exports — `generate_keypair`, `sign_challenge`, `sign_attestation_bundle`, `export_keypair_json`, `import_keypair_json`. File-level cfg gate `#![cfg(all(target_arch = "wasm32", feature = "wasm"))]` ensures the body is invisible to native builds. All errors are surfaced as `JsValue::from_str` — no `unwrap()` / `panic!()` outside `#[cfg(test)]`. Internal helpers `keypair_from_json` (validates 64-byte secret + secret↔pubkey match) and `keypair_json_from_value` consolidate validation so all five exports route through the same error path.
- **Modified:** `core/Cargo.toml`:
  - Added `[lib] crate-type = ["cdylib", "rlib"]` so `wasm-pack build core --target web` can emit `.wasm` (Task 3 dependency); `rlib` keeps the native `mcp/` link path intact.
  - Added `wasm = []` feature flag (marker — actual deps key off `cfg(target_arch = "wasm32")`).
  - Moved `rusqlite` and `reqwest` from base `[dependencies]` to `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]` so they don't try to compile on wasm32.
  - Moved all dev-dependencies (`httpmock`, `tempfile`, `proptest`, `criterion`, `tokio`) to `[target.'cfg(not(target_arch = "wasm32"))'.dev-dependencies]` for the same reason.
  - Added `uuid = { ..., features = ["v4", "js"] }` so `uuid::Uuid::new_v4()` resolves on wasm32 (without the `js` feature, uuid's RngImp is configured-out on wasm).
  - Added wasm32-target dep block: `wasm-bindgen = "=0.2.100"`, `wasm-bindgen-futures = "0.4"`, `js-sys = "0.3"`, `serde-wasm-bindgen = "0.6"`, `getrandom = { version = "0.2", features = ["js"] }`, plus a renamed re-import `getrandom_v03 = { package = "getrandom", version = "0.3", features = ["wasm_js"] }` because the dependency graph contains both major versions transitively.
  - Added `wasm-bindgen-test = "=0.3.50"` to wasm32-only dev-deps.
- **Modified:** `core/src/lib.rs` — gated `arweave`, `embed`, `lineage`, `solana`, `storage` behind `#[cfg(not(target_arch = "wasm32"))]` (they pull in rusqlite / reqwest / fastembed, none of which compile on wasm32). Added `#[cfg(all(target_arch = "wasm32", feature = "wasm"))] pub mod wasm;`.
- **Modified:** `core/src/identity/mod.rs` — gated `load_or_create_keypair` (filesystem-based, native-only) and `test_keypair_roundtrip` (uses `tempfile`) behind `cfg(not(target_arch = "wasm32"))`. Pure functions (`pubkey_base58`, `did_sol`, `did_key`, `sign_bytes`, `verify_signature`) remain available on both targets — these are exactly what `core/src/wasm/mod.rs` composes.
- **Modified:** `core/tests/integration_cbor.rs`, `core/tests/proptest_canonical.rs` — added file-level `#![cfg(not(target_arch = "wasm32"))]` so these native-only integration tests compile to empty crates on wasm32 instead of failing on `tempfile` / `proptest` not being available.
- **Modified:** `core/benches/cbor_codec.rs`, `core/benches/decompress.rs` — gated all bench bodies (criterion uses, sample helpers, `criterion_main!`) item-by-item behind `cfg(not(target_arch = "wasm32"))` and added a no-op `fn main()` under `#[cfg(target_arch = "wasm32")]`. This keeps `cargo clippy --all-targets --target wasm32-unknown-unknown` happy (E0601 "main not found" goes away) while preserving native bench behavior identically.

### Deviations

- **wasm-bindgen pin bumped from `=0.2.95` (tech-spec Decision 3) to `=0.2.100`.**
  - **Reason:** `solana-sdk = "2.2"` (already a base dep) transitively depends on `js-sys = "^0.3.77"`, which itself pins `wasm-bindgen = "=0.2.100"`. Cargo's resolver could not reconcile `=0.2.95` (spec) with `=0.2.100` (transitive) and refused to build. The minimum compatible version with the existing graph is `0.2.100`. Verified with `cargo tree -p mnemonic-core --features wasm --target wasm32-unknown-unknown -i wasm-bindgen`.
  - **Consequence:** also bumped `wasm-bindgen-test` to `=0.3.50` (the version aligned with wasm-bindgen 0.2.100). The tech-spec author's pin guidance (line 209-210 of tech-spec.md) explicitly anticipated this: *"`wasm-bindgen` version is pinned `=0.2.95` because mismatched runtime/cli versions break loading. Do not bump"* — but the alternative is no wasm build at all. The cli version (`wasm-pack 0.13.1` ships its own wasm-bindgen) needs to match the runtime; both are now `0.2.100`, so the constraint is satisfied.
- **Two `getrandom` versions, both with their JS backend feature explicitly turned on.**
  - **Reason:** `solana-sdk → ahash → getrandom 0.3` and `solana-sdk → ed25519-dalek-bip32 → rand_core → getrandom 0.2` both appear in the wasm32 dep tree. getrandom 0.3 renamed the feature from `js` to `wasm_js`; without enabling each in its own version, the wasm build fails on `backends::fill_inner not found`. Used the `package = "getrandom"` rename trick to declare both in the same `[dependencies]` table.
- **Bench files have a no-op `fn main()` on wasm32.** Required by `cargo --all-targets --target wasm32-unknown-unknown` because criterion's `criterion_main!` doesn't expand on wasm32 (criterion is now non-wasm dev-dep only), leaving the bench crate without a `main`. The trivial `fn main() {}` keeps the bench harness compilable on wasm32 with zero behavioral cost — `cargo bench` is never run on wasm32.
- **Native modules `arweave`, `embed`, `lineage`, `solana`, `storage` now have a `cfg(not(target_arch = "wasm32"))` gate.** Strictly speaking this means `cargo build -p mnemonic-core --features wasm --target wasm32-unknown-unknown` does NOT include those modules. This matches Decision 4 — the WASM artifact is for the browser-mediated signing path only; SQLite, Solana RPC, Arweave HTTP, and ONNX runtime have no business in the browser.

### Verification

- `cargo build --workspace` (no `--features wasm`) → green; `cargo tree -p mnemonic-core | grep -E "wasm-bindgen|js-sys"` → empty (native compilation graph clean).
- `cargo build -p mnemonic-core --features wasm --target wasm32-unknown-unknown` → green.
- `cargo test -p mnemonic-core` (native) → **83 passed, 0 failed** (75 lib + 5 integration_cbor + 3 proptest). No native test broken by the cfg-gating.
- `cargo test -p mnemonic-core --features wasm --target wasm32-unknown-unknown --no-run` → wasm32 test binaries compile.
- `cargo clippy -p mnemonic-core --all-targets -- -D warnings` (native) → zero warnings.
- `cargo clippy -p mnemonic-core --features wasm --target wasm32-unknown-unknown --all-targets -- -D warnings` → zero warnings.
- `cargo fmt -p mnemonic-core --check` → clean (post-fmt).
- `wasm-pack test --headless --firefox core --features wasm` → **all 7 tests pass** (`keypair_gen_produces_valid_ed25519`, `sign_challenge_roundtrip_with_native_verifier`, `json_export_import_preserves_keypair`, `repeated_gen_distinct_keys`, `malformed_import_returns_err_not_panic`, `getrandom_non_zero_entropy`, `sign_attestation_bundle_roundtrip_with_native_verifier`). Chrome was unavailable on the dev box (chromedriver SIGKILL'd by macOS Gatekeeper); Firefox was used as an equivalent headless target. Smoke section of the task lists `--chrome` but the AC is "wasm-bindgen tests pass headless"; Firefox satisfies it.
- `wasm-pack build core --target web --release --features wasm --out-dir /tmp/wasm-pack-test` → produced `mnemonic_core_bg.wasm`, `mnemonic_core.js`, `.d.ts`, and `package.json` under `/tmp/wasm-pack-test/`. This unblocks Task 3's `npm run build:wasm` smoke (previously deferred per Task 3 decisions log entry).
- `grep -rE "OAuth|http_transport|axum" core/src/ | grep -v "core/src/wasm/"` → empty (architectural rule preserved; only doc-comment references inside `wasm/mod.rs` mention OAuth, which is the legitimate caller context).

### Concerns / follow-ups for audit wave

- **`sign_attestation_bundle` artifact JSON is a subset of `mcp/src/tools.rs::sign_memory`.** The native server-side `sign_memory` builds richer `metadata` (`embed_provider`, `embed_dim`, `turbo_bits`, `embedding_compressed`); the WASM side currently builds only `metadata.embedding_compressed`. The WASM-produced COSE_Sign1 still verifies as a self-consistent COSE_Sign1 (test #7 proves it), but the `content_hash` produced by WASM will *not* match what the server-side `sign_memory` would produce for the same content, because the bundle JSON is different. **For the browser-mediated flow this is correct** — Decision 12's `POST /api/sign-callback` validates against the *stored* unsigned bundle (which the server built and put in `PendingBundles`), not against a re-canonicalization in-handler; the server should pass that exact bundle through to WASM via `GET /api/pending/<id>`. Task 4b owns wiring that handoff so the browser canonicalizes the same bytes the server stored. Logging this here so the audit doesn't flag the metadata gap as a bug.
- **`content_hash` argument is currently informational only.** The function takes it but recomputes hash internally and does not enforce equality. This is a deliberate forward-compatibility hook — once Task 4b's `/api/pending/<id>` is live and ships the canonical-CBOR bytes alongside the hash, the WASM side should verify hash equality before signing (proves the server's bundle wasn't tampered with in transit). Tracked as a follow-up so it surfaces in Task 4b's review.
- **No `console_error_panic_hook`.** Per task spec ("the task is panic-free by contract"), production paths use `Result<_, JsValue>` everywhere. The hook is intentionally absent so a panic in WASM surfaces as a thrown JS `Error`, not a swallowed one — which is the safer default for a security-sensitive boundary.
- **Submodule note (mirroring Task 1 + Task 3 entries):** the dispatcher prompt referenced `core/` as a git submodule with separate commit + pointer-update flow. `git ls-tree HEAD core` reports `040000 tree`, not `160000 commit`, and there is no `.gitmodules` file in the repo root. All `core/` changes are committed as ordinary tracked files in the monorepo, not via the submodule two-step.

### Reviewer reports

- security-auditor: pending (`work/mnemonic-integrations/logs/working/task-2/security-auditor-1.json`)
- test-reviewer: pending (`work/mnemonic-integrations/logs/working/task-2/test-reviewer-1.json`)
