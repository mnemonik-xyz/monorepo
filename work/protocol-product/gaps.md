# Code gap register

Status: reviewed at `89532db4556e53e81b5243c57535a76046c08207` on 2026-10-02.
Audit method: static source and test inspection. No runtime or production test ran during this audit.
This review includes implementation PR #265 and planning PR #266.
See [baseline evidence](baseline-audit.md) for test references, public claims and release limits.
The findings below describe paths inspected, not every possible execution path.

Source links use repository paths. Findings refer to the baseline revision above.
New implementation commits require another review.

| ID | Observed code | Target and closure evidence | Work |
|---|---|---|---|
| F1 | [SDK signMemory](../../packages/sdk/src/client.ts) sends content for a server-built signing bundle | Private artifact construction stays local; test server receives no plaintext or recall key | Task 2 |
| F2 | [SDK sealMemory](../../packages/sdk/src/client.ts) posts unsigned outer CBOR inside JSON; [anchor route](../../mcp/src/sealed_routes.rs) expects raw signed COSE. SDK producer uses `did:key:` with base58; route checks `did:sol:` | One byte contract and author identity across SDK, route and core; real WASM integration | Task 2 |
| F3 | [sealed routes](../../mcp/src/sealed_routes.rs) store hosted sealed data and use local write mode | Anchored path stores metadata only; inspect database after delivery and failure | Task 2 |
| F4 | [sealed routes](../../mcp/src/sealed_routes.rs) lack shared payment, quota and verified-delivery coordination. The direct anchor route still uploads and submits a Solana memo | Every hosted write passes the same gate; invalid input has no charge or upload | Tasks 2, 3 |
| F5 | [hosted recall](../../mcp/src/tools.rs), `recall_with_hosted_rk`, opens content and embeddings | Target private recall runs locally; remove or explicitly retire hosted decryption path | Task 2 |
| F6 | Superseded implementation finding: [ingest_a2a](../../mcp/src/tools.rs) now verifies client signatures, stores original bytes externally and retains metadata receipts. SDK recovery bypasses MCP | Partial evidence from #265; complete existing acceptance and live discovery review before closure | Existing sealed tasks 18–21 |
| F7 | [enumerate_anchored](../../core/src/restore/mod.rs) drops failed source results through `if let Ok` | Return source diagnostics; distinguish outage from empty result | Task 4 |
| F8 | [fetch_restorable](../../core/src/restore/mod.rs) uses the standard self-describing row path; it does not dispatch the existing `rebuild_sealed_row` helper | Dispatch supported verified kinds; sealed recovery and explicit unsupported-kind tests | Task 4 |
| F9 | [build_merkle_commitment](../../mcp/src/tools.rs) commits SQL hashes and gives inclusion evidence | Remove completeness claims; owner-authenticated checkpoint tests define bounded completeness | Tasks 4, 6 |
| F10 | [perform_delivery_check](../../mcp/src/tools.rs) includes a SQL row check and lacks expected original bytes and author parameters | External verification survives receipt failure; compare exact bytes and author; expose separate persistence diagnostics | Task 2 |
| F11 | [paid artifact staging](../../mcp/src/paid_artifact.rs) stores COSE, canonical CBOR, content and embeddings in SQL for retries | Client resubmission bound to digest; crash/retry test without hosted artifact bytes | Task 3 |
| F12 | [SDK recall](../../packages/sdk/src/client.ts) sends `top_k`; MCP reads `limit`. SDK drops proof fields and expects different total/time fields | Preserve relevant evidence and align argument contract; SDK-to-dispatch test | Tasks 2, 6 |
| F13 | Public docs retain conflicting memo, hosted local, payment and receipt claims. The [decoupling sequence](../DECOUPLING-SEQUENCE.md) has an unclosed live gate | Use the [claim map](baseline-audit.md); update public docs and record gate evidence without inferring closure | Tasks 5, 6; chain-agnostic T2 |

F1–F5 and F7–F13 remain open. F6 has merged implementation with incomplete acceptance.
The [baseline audit](baseline-audit.md) names functions and existing tests at this revision.
No finding closes merely because its description changed.

## Corrections to earlier economic findings

Tokenomics F1 says only anchored `sign_memory` is paid.
The dispatcher also gates `mnemonic_attest_a2a` on x402 deployments.
Tokenomics F2 states payment after delivery without a rail qualification.
Payment acceptance or settlement can precede delivery on some paths.
Both claims require the qualification in [tech-spec.md §5](tech-spec.md#5-separate-payment-and-delivery-states).

## Closure record

For each finding, append implementation commit, test command, result and remaining limitation.
Record external provider smoke evidence separately from mocked tests.
Mark a finding closed only when its acceptance condition passes.
Do not close task 14, #242 or #233 from this table alone.
Follow the existing A2A contract's wider closure requirements.

## Operational gaps

Customer demand, retention requirements, key backup usability and sustained availability remain unmeasured here.
Customer pilots and recovery drills must supply that evidence.
Current backend discovery remains a provider dependency even after MCP SQL dependency ends.


## Implementation follow-up — `9fc8e42`

The table above remains the historical baseline. The [implementation evidence matrix](implementation-evidence.md) records the subsequent code review and executed tests.
F1–F5, F7–F8 and F10–F12 now have implementation evidence for the declared client-prepared ingestion, native recovery and retry interfaces.
This is not a blanket migration claim: legacy server-prepared signing and undrained historical paid staging remain explicit exceptions.
F6 retains the existing A2A tasks' wider acceptance ownership even though the final explicit SQL-loss integration test passed.
F9's technical checkpoint support is implemented; completeness claims still require accurate source/release documentation.
F13 remains a release-documentation/live-provider gate. The new source capability matrix does not establish deployment or published-package readiness.
