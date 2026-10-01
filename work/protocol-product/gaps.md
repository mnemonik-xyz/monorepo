# Code gap register

Status: open implementation findings at `44fc172e3afa0efcd3beff2fa555b05dd8583eec`.
Audit method: static source inspection. No runtime or production test ran for this package.
PR #263 changes specifications only. Its parent is the earlier audited `a8f34fc` baseline.
The findings below describe paths inspected, not every possible execution path.

Source links use repository paths. Findings refer to the baseline revision above.
New implementation commits require another review.

| ID | Observed code | Target and closure evidence | Work |
|---|---|---|---|
| F1 | [SDK signMemory](../../packages/sdk/src/client.ts) sends content for a server-built signing bundle | Private artifact construction stays local; test server receives no plaintext or recall key | Task 2 |
| F2 | [SDK sealMemory](../../packages/sdk/src/client.ts) posts JSON outer CBOR; [sealed route](../../mcp/src/sealed_routes.rs) expects a verifiable signed body | One byte contract and author identity across SDK, route and core; real WASM integration | Task 2 |
| F3 | [sealed routes](../../mcp/src/sealed_routes.rs) store hosted sealed data and use local write mode | Anchored path stores metadata only; inspect database after delivery and failure | Task 2 |
| F4 | [sealed routes](../../mcp/src/sealed_routes.rs) lack the shared paid/quota delivery coordinator | Every hosted write passes the same gate; invalid input has no charge or upload | Tasks 2, 3 |
| F5 | [hosted recall](../../mcp/src/tools.rs), `recall_with_hosted_rk`, opens content and embeddings | Target private recall runs locally; remove or explicitly retire hosted decryption path | Task 2 |
| F6 | [attest_a2a](../../mcp/src/tools.rs) signs with operator key and uses SQL storage; recall uses SQL | Client authorship and external unchanged-byte storage; SQL-loss and continuation tests | Existing sealed tasks 18–21 |
| F7 | [enumerate_anchored](../../core/src/restore/mod.rs) drops failed source results through `if let Ok` | Return source diagnostics; distinguish outage from empty result | Task 4 |
| F8 | [fetch_restorable](../../core/src/restore/mod.rs) uses the standard self-describing row path | Dispatch supported verified kinds; sealed recovery and explicit unsupported-kind tests | Task 4 |
| F9 | [build_merkle_commitment](../../mcp/src/tools.rs) commits SQL hashes and gives inclusion evidence | Remove completeness claims; owner-authenticated checkpoint tests define bounded completeness | Tasks 4, 6 |
| F10 | [perform_delivery_check](../../mcp/src/tools.rs) includes a SQL row existence check | External verification survives receipt failure; expose separate persistence diagnostics | Task 2 |
| F11 | [paid artifact staging](../../mcp/src/paid_artifact.rs) stores signed bytes in SQL | Client resubmission bound to digest; crash/retry test without hosted artifact bytes | Task 3 |
| F12 | [SDK recall](../../packages/sdk/src/client.ts) reduces evidence and sends `top_k`; MCP uses `limit` | Preserve relevant evidence and align argument contract; SDK-to-dispatch test | Tasks 2, 6 |
| F13 | [CLAUDE.md](../../CLAUDE.md) and [decoupling sequence](../DECOUPLING-SEQUENCE.md) contain legacy memo and payment claims | Update public behavior after code changes; reviewed claim-to-test matrix | Task 6 |

Inspect these functions at the pinned revision:
`client.ts` lines 256, 426 and 538; `tools.rs` lines 1742, 2458, 3100 and 4619;
`restore/mod.rs` lines 78 and 109; `paid_artifact.rs` line 172.
Line numbers are navigation hints, not stable interfaces.

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
