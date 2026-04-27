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
