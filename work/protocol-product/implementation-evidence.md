# Implementation and acceptance evidence

Status: implementation evidence collected on 2026-10-02. This record does not close release acceptance.
Implementation revision: `9fc8e42` (`feat(protocol): add client-owned delivery recovery and storage portability`).
This record describes that implementation; later code changes require their affected checks to be refreshed.

Environment: local macOS arm64, native Rust, Node, rebuilt WebAssembly and local mock HTTP services.
Cryptographic checks use real signatures, encryption and grants unless a test explicitly uses mocked bindings.
Storage and payment fixtures do not establish live provider behavior.

[Task 5](tasks/5.md) requires integrated acceptance after existing sealed-memory tasks 18–21 pass.
Those tasks retain ownership of A2A acceptance.
Their wider task 14, #242 and #233 closure conditions remain separate.

## Acceptance matrix

“Observed” below means a local test demonstrated the named scope.
It does not mean every client, backend, payment rail or deployment passed.

| Acceptance | Observed evidence | Remaining limit or release condition |
|---|---|---|
| A-1 Offline local | [A2A recovery tests](../../packages/sdk/test/a2a.recovery.test.ts) sign and recall without network. SDK private-ingestion test also exercises local sealed store/open | CLI real-WASM tests also reopen durable local ciphertext without login. CLI sealed recall is unsupported; SDK local recall requires a caller-provided embedder. Default SDK indexes are session-only |
| A-2 Private ingestion | [Signed-ingestion tests](../../mcp/tests/signed_ingestion.rs) contain real SDK/WASM HTTP coverage for raw signed sealed bytes, no plaintext request, local open and public consent | Explicit SDK HTTP tests passed. Legacy server-prepared writes remain a disclosed migration interface |
| A-3 Hosted storage | Signed-ingestion and [A2A tests](../../mcp/tests/integration_a2a_mcp_tools.rs) inspect metadata-only storage after delivery, retry and failure. New payment operations retain metadata | Historical paid staging can retain payloads until verified client resubmission drains them. Backups and journals need operational retention treatment |
| A-4 Delivery | `exact_delivery_retry_and_receipt_failure_are_independent`, invalid-author and tampered-byte tests pass in the workspace run | Real storage is mocked. A successful fetch is a point-in-time observation, not sustained availability |
| A-5 Gate coverage | Invalid author, envelope and hosted-local inputs fail before uploads. Unsupported paid A2A rails fail without upload or settlement | Shared client-prepared ingestion has bounded support. Do not infer full rail or legacy-surface equivalence from these tests |
| A-6 Financial retry | Settlement-crash, concurrent retry, known-locator upload recovery and terminal-remedy tests pass. Approval status tests distinguish delivery from settlement | Financial fixtures seed durable accepted receipts. No live settlement, refund or credit occurred. Durable accounting loss remains an incident |
| A-7 General restore | [Native recovery tests](../../core/tests/recovery_checkpoint.rs) restore exact public/sealed bytes and complete plaintext. Source errors, partial pages and budgets remain visible | Native client API supports memory.v1 and sealed.v1, including existing sealed JSON. Legacy index command cannot open sealed memory without keys |
| A-8 A2A SQL loss | Existing `sdk_http_sealed_end_to_end` deletes receipts, disables HTTP MCP and restores with a fresh SDK process | Explicit SDK/WASM HTTP run passed after the final workspace suite; external storage/index are mocked |
| A-9 Continuation | `parent_validation_survives_receipt_loss` passes. New operator-continuation test uses externally verified migrated parent bytes | The separate ignored SDK/WASM SQL-loss test also passed explicitly. Preserve parent-author eligibility and configured-origin limits |
| A-10 Completeness | Native graph tests cover forks, missing parents, cycles and unpinned scope. SDK tests cover lag, source replacement, budgets and pinned ancestry | Native memory IDs require independently authenticated digest/author pins for every ancestor. No scan proves all history or newest heads |
| A-11 Key backup | [Checkpoint tests](../../packages/sdk/test/checkpoint.test.ts) restore a key and signed checkpoint into a fresh client. Wrong owner, tampering, wrong password and outsider opening fail | Separate recipient encryption keys require explicit backup. Production key-backup usability and disaster drills remain unmeasured |
| A-12 Operator switch | `independent_operator_continues_from_migrated_parent_without_source` passes with a fresh operator key, empty receipts and stopped source listener | This test uses a plain signed A2A parent and mocked storage. Sealed-stream migration and recipient open are tested separately, not one deployed end-to-end drill |
| A-13 Live source | No new live discovery evidence was generated in this implementation session | Prior A2A evidence records HTTP 403. Supported live scans, exact fetches, lag windows and the chain-agnostic parity gate remain required |
| A-14 Customer drill | No customer or paid-pilot evidence was generated | Outreach, paid recovery drills, migration repetition and measured support costs remain outstanding |
| A-15 Index replacement | [Discovery tests](../../packages/sdk/test/discovery.test.ts) and A2A recovery tests exercise previous-bundler and Arweave query variants, source replacement, forged hints and disabled discovery | Both provider services are mocked. Production availability of interchangeable indexes is not established |
| A-16 Backend migration | [Storage-portability tests](../../packages/sdk/test/storage-portability.test.ts) copy exact bytes, disable the source, restore forks through destination-only configuration and open as recipient | Scope is signed A2A with embedded grants and completed streams. General-memory migration, standalone grants and SSE-event migration are unsupported |
| A-17 Release drill | SDK source build and local tests provide implementation evidence | No pinned published-package install, staging release, operational rollback or paid pilot release drill has passed in this record |

A-16 uses an owner-signed locator manifest bound to an authenticated checkpoint.
The [adapter contract](../../docs/storage-portability.md) declares Arweave fetch and configured HTTP object storage capabilities.
Both storage services are mocks in the test.
Blob restoration uses the standalone manifest API; generic import remains on its Arweave gateway path.

## Recorded commands and outcomes

Run these commands from the repository root.
Finish WebAssembly builds before SDK tests; the build replaces generated test artifacts.
Run native doctests without concurrent Cargo builds that replace shared crate outputs.

| Command | Recorded result and scope |
|---|---|
| `npm run build --workspace @mnemonik-xyz/sdk` | Passed in the inspected SDK build log; includes TypeScript, WebAssembly and browser output |
| `npm test --workspace @mnemonik-xyz/sdk` | Latest run passed 365 tests across 29 files, including five storage-portability tests |
| `npm test --workspace @mnemonik-xyz/sdk -- --run test/storage-portability.test.ts` | Five passed after budget, buffer-snapshot and deadline fixes; real WASM, mocked storage |
| `npm test --workspace @mnemonik-xyz/sdk -- --run test/checkpoint.test.ts` | Seven passed; authenticated scope, encrypted backup, fresh client, wrong keys and digest bindings |
| `npm test --workspace @mnemonik-xyz/sdk -- --run test/discovery.test.ts test/a2a.recovery.test.ts` | Task 9 records 21 passing tests with real WASM and mocked providers |
| `npx tsc -p packages/sdk/tsconfig.json --noEmit` | Passed during focused checkpoint and storage checks |
| `cargo test -p mnemonic-core --test recovery_checkpoint --test integration_restore` | Fifteen passed: six new recovery tests and nine existing restore tests |
| `cargo test -p mnemonic-core --test storage_portability` | One passed: configured destination parent fetch, digest mismatch, disabled backend and unsafe origin rejection |
| `cargo test -p mnemonic-core --lib arweave::graphql -- --nocapture` | Eleven passed |
| `cargo test -p mnemonic-core --lib solana::tests::list_memo_anchors -- --nocapture` | One passed. Existing unrelated test-helper dead-code warning was emitted |
| `cargo clippy -p mnemonic-core --lib -- -D warnings` | Passed during recovery and storage implementation |
| `cargo test -p mnemonic-mcp --features test-support --test signed_ingestion` | Workspace execution passed six tests and skipped two SDK HTTP tests. Both skipped tests passed in a separate explicit run |
| `cargo test -p mnemonic-mcp --features test-support --test integration_a2a_mcp_tools` | Workspace execution passed seven tests, including independent-operator continuation; one SDK/WASM recovery test was skipped |
| `cargo test -p mnemonic-mcp --features test-support --bin mnemonic-mcp delivery_status_tests` | Three passed, including malformed historical receipt rejection |
| `cargo test -p mnemonic-mcp --features test-support --test integration_a2a_mcp_tools independent_operator_continues_from_migrated_parent_without_source -- --nocapture` | One passed in a separate focused run |
| `cargo test -p mnemonic-mcp --features test-support --lib delivery_operation` | Four passed before the final receipt-binding patch |
| `cargo test -p mnemonic-mcp --features test-support --lib paid_artifact` | Eight passed before the final receipt-binding patch |
| `cargo test -p mnemonic-mcp --features test-support --lib quota_reservation_tests` | Three passed |
| `cargo test -p mnemonic-mcp --features test-support --test sign_callback` | Six passed |
| `cargo test -p mnemonic-mcp --features test-support --test free_anchor_quota a_sql_only_anchor_receipt` | Corrected receipt-only assertion passed; all fourteen quota tests then passed in the final workspace run |
| `git diff --check` | Passed during focused recovery and storage review |

### Explicit SDK HTTP checks

These tests are ignored by ordinary Rust test commands.
They require the built SDK and real WebAssembly files.
Record explicit execution independently of the workspace counts above.

```sh
cargo test -p mnemonic-mcp --features test-support --test signed_ingestion sdk_ -- --ignored --nocapture
cargo test -p mnemonic-mcp --features test-support --test integration_a2a_mcp_tools sdk_http_sealed_end_to_end -- --ignored
```

The first command passed both `sdk_wasm_real_http_private_ingestion` and `sdk_http_recall_limit_and_evidence`.
The second command passed `sdk_http_sealed_end_to_end` after the final workspace suite (one test).
Both explicit commands used the rebuilt real SDK/WASM and mocked external services.

## Final regression

All checks below passed at implementation revision `9fc8e42`:

| Check | Result |
|---|---|
| `CARGO_INCREMENTAL=0 cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support` | 1,305 passed, zero failed, seven ignored across 76 test groups; doctests completed |
| Explicit signed-ingestion SDK HTTP command above | Two passed |
| Explicit A2A SQL-loss SDK HTTP command above | One passed |
| SDK complete suite | 365 passed across 29 files |
| CLI complete suite | 190 passed, two skipped; real-WASM disk reopening and hash-substitution rejection included |
| SDK build, CLI build and `npx tsc -b packages/sdk packages/cli` | Passed |
| Core library strict Clippy and MCP library/binary strict Clippy | Passed during implementation review |
| `git diff --check` and local Markdown target checks | Passed |

The first integrated run exposed a stale SQL-only receipt expectation and concurrent-build doctest failures.
The final sequential run passed after correcting the receipt test and rebuilding consistently.
An intervening disk-space failure was resolved by deleting only generated Rust incremental caches; the final run disabled incremental compilation.
CLI regressions led to durable local ciphertext storage, token-free local opening, hash verification, and explicit rejection of unsupported hosted paths before network disclosure.

Temporary session logs include `mnemonik-final-workspace-tests.log`, `mnemonik-final-sdk-http.log`, and `mnemonik-final-a2a-http.log`.
They are local traces, not published release artifacts; the commands above reproduce the checks.

## Remaining release work

Complete the existing sealed tasks 18–21 acceptance review without duplicating ownership.
Run supported live-source scans and measure index lag with redacted configuration.
Complete the independently configured operator and published-package release drills.
Preserve financial replay records during backup and rollback.
Keep product demand, customer evidence and release readiness separate from implementation test success.
