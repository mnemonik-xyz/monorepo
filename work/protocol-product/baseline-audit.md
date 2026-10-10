# Implementation baseline and claim map

Reviewed on 2026-10-02 at `89532db4556e53e81b5243c57535a76046c08207`.
This baseline includes A2A implementation #265 and planning changes #264 and #266.
Method: three parallel source audits, followed by consolidation against the code and task contracts.
Test names below identify existing coverage. They do not report fresh test runs.
No runtime, production, payment or live-provider drill ran during this documentation audit.

## Findings and existing tests

| Finding | Current code and evidence | Remaining work |
|---|---|---|
| F1 | SDK `signMemory` sends `{content}` to MCP in [client.ts](../../packages/sdk/src/client.ts). The server prepares the artifact | Task 2: client preparation; legacy migration disclosure |
| F2 | SDK `sealMemory` sends JSON containing unsigned `outer_cbor`. [anchor_sealed_handler](../../mcp/src/sealed_routes.rs) expects raw signed COSE and a different producer DID form | Task 2: align transport and identity; real SDK/WASM-to-route coverage |
| F3 | Both sealed handlers call `save_sealed_attestation` with the full body and `WriteMode::Local`. [SQLite](../../core/src/storage/sqlite.rs) retains `sealed_blob` | Task 2: hosted metadata-only persistence, including failures |
| F4 | Direct sealed routes upload without shared quota, payment or read-back verification. The nonlocal anchor branch also submits a Solana memo | Tasks 2–3: shared validation and delivery coordination |
| F5 | [recall_with_hosted_rk](../../mcp/src/tools.rs) decrypts embeddings and content. [HTTP dispatch](../../mcp/src/mcp.rs) and [session creation](../../mcp/src/api.rs) remain reachable | Task 2: retire this path or explicitly migrate it |
| F6 | [ingest_a2a](../../mcp/src/tools.rs) verifies client-signed bytes, external parents and exact fetched bytes. SQL stores receipts; SDK recovery bypasses MCP | Existing sealed tasks 18–21: acceptance remains incomplete |
| F7 | [enumerate_anchored](../../core/src/restore/mod.rs) discards both source errors with `if let Ok` | Task 4: distinguish empty, partial and failed scans |
| F8 | `fetch_restorable` calls `rebuild_row_self_describing`, not the existing [rebuild_sealed_row](../../core/src/rebuild.rs) helper | Task 4: verified-kind dispatch and unsupported-kind diagnostics |
| F9 | [build_merkle_commitment](../../mcp/src/tools.rs) commits SQL-owned hashes. A2A uses caller-pinned heads, but its cursor checkpoint is unsigned resume state | Tasks 4 and 6: authenticated backup and bounded claims |
| F10 | `perform_delivery_check` verifies external COSE/hash, then requires a SQL row. It lacks original-byte and expected-author parameters | Task 2: independent exact-byte delivery verification |
| F11 | [paid_artifact.rs](../../mcp/src/paid_artifact.rs) persists COSE, canonical CBOR, content and embedding context. `resume_due_paid_deliveries` reuses these bytes | Task 3: digest-bound client resubmission; retain financial replay state |
| F12 | SDK sends `top_k`; MCP reads `limit`. SDK drops evidence and expects `total`/`signed_at`, versus server `total_attestations`/`created_at` | Tasks 2 and 6: actual dispatch contract and evidence preservation |
| F13 | Public documents conflict with these paths and the recorded gate status | Task 6: public corrections; task 5 and chain-agnostic T2: live evidence |

Existing tests are narrower than the target acceptance:

- [SDK sealed tests](../../packages/sdk/test/sealed.test.ts): `sealMemory + openMemory round-trip (mock WASM)` uses mocked bindings and fetch.
- [Sealed route tests](../../mcp/tests/sealed_routes.rs): `anchor_sealed_rejects_wrong_producer` and `store_sealed_never_writes_embedding_row` do not exercise real SDK transport.
- [SDK recall tests](../../packages/sdk/test/integration/recall-verify.test.ts): `returns hits with similarity + tags` uses a mock server.
- [Delivery tests](../../mcp/tests/delivery_guarantee.rs): `happy_path` and `stdio_anchored_demotes_on_refetch_failure` do not establish receipt-independent verification.
- [Recall session tests](../../mcp/tests/recall_session.rs) cover the retained hosted-key mechanism, not its retirement.
- [Paid staging tests](../../mcp/src/paid_artifact.rs): `staged_envelope_is_immutable_for_a_correlation_id` and `delivery_context_round_trips_without_reembedding` verify existing SQL staging.
- The same module tests `delivery_retry_reuses_recorded_arweave_progress_without_a_new_claim` and `staged_cose_hash_matches_paid_operation_artifact_hash`.
- [A2A integration tests](../../mcp/tests/integration_a2a_mcp_tools.rs): `original_bytes_remote_metadata_only_and_retry`, `parent_validation_survives_receipt_loss`, and `failed_delivery_is_pending_and_retry_uses_existing_remote_bytes`.
- [A2A recovery tests](../../packages/sdk/test/a2a.recovery.test.ts) cover local offline use, pinned heads, forged hints, bounds, source errors and delayed-parent resumption.

## A2A implementation and acceptance

Implementation commit: `979c590388bfa1c9929661bb72f0a763d4742a4d` (#265).
Existing [sealed tasks 18–21](../sealed-memories/tasks/18.md) retain implementation ownership and remain `in_progress`.
The former F6 description of operator signatures and SQL artifact storage no longer describes this path.
Hosted recall still uses receipt metadata. Independent restoration uses the SDK's external discovery and local index.

The `sdk_http_sealed_end_to_end` integration test requires built SDK and real WebAssembly artifacts.
It is ignored by default and must be explicitly enabled.
It deletes A2A receipts, stops HTTP MCP, restores through a fresh client and restarts HTTP before continuation.
The external artifact and index service is mocked. The test reuses the server state object after restart.
It does not establish a separate operator deployment or recovery after financial database loss.

[Prior validation](../../docs/sealed-a2a.md) records passing targeted tests and an explicit HTTP/WASM run.
Those are historical implementation results, not new results from this audit.
That record reports HTTP 403 from live previous-bundler discovery; production recovery remains unverified.
The full negative-parent, fork, duplicate-locator, timeout and operator-rotation matrix still needs mapped acceptance evidence.
Actual SSE streaming (#61) remains required for the wider streaming scope.
DID discovery (#74), or an explicitly agreed AgentCard-only V1 scope, remains a separate obligation.
Do not close task 14, #242 or #233 from this review.

## Claim-to-release map

| Claim | Current evidence or exception | Required work |
|---|---|---|
| Offline local memory (A-1) | A2A local signing/recall has offline tests; SDK defaults to a session-only index; CLI provides durable indexing | Tasks 2 and 5: declare each supported client and kind |
| Client-private ingestion (A-2) | A2A has client sealing; general memory preparation and hosted recall retain server access | Task 2; F1, F2, F5 |
| Metadata-only hosted storage (A-3) | A2A receipts meet the narrow storage boundary; direct sealed routes and paid staging retain payloads | Tasks 2–3; F3, F11 |
| Exact verified delivery (A-4) | A2A fetch-compares bytes; general delivery still depends on SQL | Task 2; F10 |
| Shared gates (A-5) | Direct sealed routes bypass the coordinator | Tasks 2–3; F4 |
| Financial retry (A-6) | Durable replay and staging tests exist. Universal Paywall settlement can precede upload | Task 3: byte-free retries and explicit remedy states |
| General restore (A-7) | Standard memory reconstruction exists; source errors and sealed dispatch remain incomplete | Task 4; F7–F8 |
| A2A SQL-loss recovery and continuation (A-8, A-9) | Real cryptography with mocked external service and receipt deletion | Existing tasks 18–21, then task 5 |
| Bounded completeness (A-10) | A2A checks pinned-head ancestry; SQL inclusion and exhausted scans do not prove complete history | Task 4; F9 |
| Key/checkpoint backup (A-11) | Cursor resume state does not authenticate owner backup or preserve lost keys | Task 4 |
| Operator switch (A-12) | Restarted HTTP MCP is not an independent operator deployment | Task 5 |
| Production discovery (A-13) | Live source success and lag evidence are absent from reviewed records | Task 5; chain-agnostic T2 remains unresolved |
| Paying recovery customers (A-14) | Product hypothesis; no customer evidence in this package | Task 7; outreach needs explicit authorization |
| Index replacement (A-15) | Provider query variants exist; logical replacement acceptance remains planned | Task 9, reusing sealed task 20 |
| Backend migration (A-16) | Versioned authenticated locator manifest and migration acceptance remain planned | Task 10 |
| Pilot readiness (A-17) | Versioned release, staging and rollback drills remain planned | Task 12 |

## Public documentation corrections owned by task 6

| Surface | Claim to correct or qualify |
|---|---|
| [README](../../README.md) | Legacy mode names, universal memo anchoring and no-charge-on-failure promise |
| [CLAUDE.md](../../CLAUDE.md) | Contradictory HTTP local behavior; memo writes; operator never receiving text; universal payment-after-delivery |
| [AGENTS.md](../../AGENTS.md) | Nine-tool count, HTTP local writes, universal signatures and required memo verification |
| [Tools](../../docs/tools.md) and [how it works](../../docs/how-it-works.md) | Only one paid tool; legacy payment modes; universal Solana anchoring and failure demotion |
| [Whitepaper](../../docs/WHITEPAPER.md) and [yellowpaper](../../docs/YELLOWPAPER.md) | New-write memo timestamps, payment guarantees and scope of independent recovery |
| [Agent card](../../webapp/public/.well-known/agent.json) and [crawler summary](../../webapp/public/llms.txt) | Every-memory signing, Solana anchoring and current tool capabilities |
| [Sealed A2A reference](../../docs/sealed-a2a.md) | MCP table claims envelope/attestation responses; implementation returns locator receipts and `receipts[]` |

These are source-document findings. This audit did not inspect deployed website content or production configuration.

## Decoupling discrepancy and next work

`1edebe7` removed memo writes from ordinary inline, sealed-anchored helper and callback paths.
The direct `anchor_sealed_handler` still submits a memo; removal is not universal.
[Stage 2 tests](../../mcp/tests/stage2_no_solana_memo.rs) use mocked sources, not measured production parity.
The [decoupling gate](../chain-agnostic/tasks/T2-stage1-gate.md) remains pending; reviewed records contain no passing live comparison.
The old sequence cannot be treated as an accurate shipped-status table.
This audit records the discrepancy without restoring memo writes, waiving the gate or claiming deployment readiness.

After task 1, tasks 2, 4, 9 and 11 can start under their existing dependencies.
Task 7 interviews require separate outreach authorization; this audit contacts nobody.
Task 3 waits for task 2. Task 10 waits for tasks 2, 4 and 9.
Task 5 also requires complete existing A2A acceptance.
Repository extraction planning does not authorize moving code or creating repositories.
