# Acceptance and release evidence

Status: acceptance requirements. See the [implementation evidence matrix](implementation-evidence.md) for executed local tests and remaining release gates. This specification alone establishes no passing result.

| Test | Required observation | Blocks claim |
|---|---|---|
| A-1 Offline local | Sign, store, open and recall without API, payment or gateway | Local independence |
| A-2 Private ingestion | Real SDK/WASM sends only complete signed sealed bytes; server receives no keys or plaintext | Private boundary |
| A-3 Hosted storage | Inspect database after success, failure and retry; no artifact, grant or vector payloads | Metadata-only operator |
| A-4 Delivery | Fetch exact bytes; reject tampering, wrong author and wrong identity | Verified delivery |
| A-5 Gate coverage | Every hosted write uses validation, quota and payment gates; invalid input has no side effects | Shared ingestion |
| A-6 Financial retry | Concurrent retry and crash use one financial operation; settled delivery failure has a visible remedy | Reliable billing |
| A-7 General restore | Sealed and public kinds restore; source outages never become empty success | Memory recovery |
| A-8 A2A SQL loss | Existing tasks 18–21 integration passes with MCP disabled and receipts deleted | A2A independence |
| A-9 Continuation | Restart configured MCP with empty receipts; append using verified external parent | SQL-independent lineage |
| A-10 Completeness | Missing parent, fork, lag and scan budget yield accurate dimensions; pinned-head ancestry passes | Bounded recovery claim |
| A-11 Key backup | Fresh client restores trusted checkpoint and keys; outsider cannot open | Usable private recovery |
| A-12 Operator switch | Independent compatible operator accepts continuation without old operator login | Operator portability |
| A-13 Live source | Record supported endpoint scan, fetch and index lag with date and config | Production discovery |
| A-14 Customer drill | Paying pilot completes migration and repeats recovery | Product demand |

Use the full adversarial matrix in the existing A2A recovery specification.
Add malformed envelopes, wrong signatures, forged grants, duplicate locators, size limits and provider timeouts for general memory.
Test receipt-write failure after external verification separately from external delivery failure.
Test financial state loss as an operational incident, not as safe replay.

## Evidence record

For each release claim, record commit, test name, command, environment, result and limitation.
Separate mocked provider tests from live provider evidence.
Record live discovery lag and observation window.
Never put client secrets or private plaintext in public evidence.
Do not claim sustained availability from a single successful fetch.

Rust implementation checks use repository feature requirements in CLAUDE.md.
SDK changes require real WebAssembly parity and SDK-to-MCP integration coverage.
Documentation-only changes require link, status and claim consistency checks.
No full Rust suite is required for this specification-only package.

## Additional portability and shipping evidence

| Test | Required observation | Blocks claim |
|---|---|---|
| A-15 Index replacement | Two discovery implementations; known-locator restore with both disabled; forged hints rejected | Indexer independence |
| A-16 Backend migration | Copy exact bytes; disable source and original operator; recover expected heads through destination | Backend portability |
| A-17 Release drill | Install pinned released artifacts; repeat save, restore, switch and continuation; preserve financial state on rollback | Pilot readiness |

A-16 requires an authenticated locator manifest and preserved original signatures.
Record which backends are supported and which results use mocks.
A-17 requires a capability matrix and explicit operational limitations.
Repository extraction alone satisfies none of these acceptance conditions.
