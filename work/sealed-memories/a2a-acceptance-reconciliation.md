# A2A acceptance reconciliation

Reviewed on 2026-10-02 against merge `7dc5079` (PR #267).
This is a source and recorded-test review, not a new runtime or live-provider run.
The [product evidence matrix](../protocol-product/implementation-evidence.md) records the passing checks at implementation revision `9fc8e42`.
The final explicit HTTP logs confirm two signed-ingestion tests and one A2A SQL-loss test passed.

Tasks 18–21 remain in progress while the specific gaps below are reconciled.
A missing acceptance assertion is not itself proof that the implementation is wrong.
Conversely, passing neighboring tests does not establish that assertion.

## Task 18: storage boundary and local recall

| Requirement | Existing evidence | Remaining check |
|---|---|---|
| Original external bytes and metadata-only receipts | `original_bytes_remote_metadata_only_and_retry` checks plain, sealed and completed-stream artifacts. SQL payload/embedding/grant tables remain empty | Preserve the historical paid-staging exception in upgrade documentation; it does not authorize new A2A payload staging |
| SDK through real HTTP dispatch, fetch and open | Explicit `sdk_http_sealed_end_to_end` uses real SDK/WASM and a separate storage/index mock | Refresh after changes to the drill; keep mock/live results separate |
| Offline local and rejected hosted local | SDK `local signing and recall require no network`; MCP invalid/local and paid-local rejection tests | Existing CLI durable-index test covers concurrent private-file writes; it is separate from HTTP recovery |
| Failed delivery, retry, rotated locators and bounds | Pending delivery is hidden; retry reuses remote bytes. SDK replacement tests deduplicate identical artifacts at distinct hinted locators | Add a real two-operator upload of identical artifact bytes. Assert different transport locators, preserved author/identity and client deduplication |
| Published behavior | [Sealed A2A guide](../../docs/sealed-a2a.md), [storage migration](../../docs/storage-portability.md), SDK and CLI guides | Review drift after final acceptance changes; current source support is not a deployed-service capability claim |

## Task 19: external parent validation

`parent_validation_survives_receipt_loss` removes receipts before valid continuation.
It covers a required locator and unavailable remote parent.
`independent_operator_continues_from_migrated_parent_without_source` adds configured blob routing, a fresh operator and changed-byte rejection.
The shared verifier contains hash, context, self-link and author/named-reader eligibility checks.

The following acceptance cases lack direct mapped end-to-end assertions in the reviewed tests:

- A valid signed parent whose hash differs from the child's signed `prev_id`.
- A valid signed parent in a different context.
- An invalid parent signature at an otherwise valid locator.
- A child signed by neither the parent author nor a named grant recipient.
- Identical valid continuation with receipts present and absent, with the same expected result.
- Exact error classification for retriable parent outage versus cryptographically invalid parent, before upload or payment side effects.
- Native parent-fetch size/deadline behavior, beyond source inspection and neighboring SDK/blob tests.

Exercise self-link and cycle guards with clearly labeled synthetic verifier inputs where needed.
Valid content-addressed A2A cycles cannot be manufactured as ordinary fixtures without solving circular hash dependencies.
Do not present the generic memory-ID cycle test as A2A cryptographic coverage.

## Task 20: discovery and verified import

Local implementation evidence is extensive:

- Provider-specific previous-bundler and Arweave queries, opaque cursors and credentials omitted from requests.
- Independently pinned authors, signed contexts, forged tags and altered signatures.
- Duplicate locators, competing branches, omission, lag and index replacement.
- Source errors, malformed pages, stalled index bodies, cancellation and scan limits.
- Resume state bound to scope and source; staged children survive until parents appear.
- Known-locator recovery with discovery disabled.

The SQL-loss HTTP script also asserts that recovery makes no MCP calls and sends no MCP authorization header to storage.
The remaining operator-rotation upload assertion belongs to task 18 and can supply its discovery fixture here.
Live enumeration, exact fetch and measured index lag remain separate evidence requirements under product A-13.
The recorded HTTP 403 is a failed observation, not a successful production discovery result.

## Task 21: restore, open and continue

The explicit SDK/WASM HTTP drill passes this sequence:

1. Write a three-artifact sealed chain, including a completed stream.
2. Delete A2A receipt and reader rows, then stop HTTP MCP.
3. Start fresh SDK processes without the old local index or token.
4. Restore pinned ancestry and open exact expected plaintext as author and recipient.
5. Reject outsider opening.
6. Restart HTTP MCP with empty receipts and append through an external parent locator.

The current script passes expected heads and plaintext through a test-process environment value.
It does not carry a signed checkpoint or expected original envelope bytes through that reset.
Separate checkpoint tests authenticate backups; separate ingestion tests compare exact envelope bytes.
Integrate these assertions into the SQL-loss drill to satisfy the full recovery specification directly.

Native sealed tests cover substituted grants, truncated streams, altered wraps and repaired-public-hash ciphertext mutations.
SDK recovery tests cover missing parents, untrusted authors, competing branches and unknown completeness without heads.
An explicit synthetic A2A graph-guard test remains unmapped.

## Product task 5 mapping

| Acceptance | Existing result | Remaining integrated evidence |
|---|---|---|
| A-8 SQL loss | Explicit real SDK/WASM HTTP drill passed with mocked external services | Carry authenticated checkpoint and original-envelope expectations through the reset |
| A-9 Continuation | Receipt-loss continuation and explicit SDK HTTP append passed | Complete task 19 adversarial and error assertions |
| A-12 Operator switch | Fresh operator accepts a migrated plain parent after source shutdown | Sealed-stream migration and recipient open are tested separately. Combine them with independent hosted continuation for the wider drill |
| A-13 Live source | No successful live acceptance claimed here | Record endpoint configuration, known artifact identities, scan/fetch outcomes and lag window without client secrets |

Reproducible local commands are listed in the [implementation evidence](../protocol-product/implementation-evidence.md#recorded-commands-and-outcomes).
Each added drill must record its revision, command, result and mock/live boundary.
The maintainer running live checks owns their separate evidence record.
This review does not close task 14, #242, #233 or any live-provider gate.

## Task 5 follow-up: local integrated results

On 2026-10-02, the task 5 working tree based on `7dc5079` passed all 11 A2A HTTP integration tests, including ignored SDK/WASM drills.
This result supersedes the corresponding missing-test observations above; it does not change the historical baseline review.
The run used real signatures, encryption, WASM and HTTP dispatch, with local mocked storage, discovery and payment endpoints.
No live provider upload or settlement occurred.

```bash
CARGO_INCREMENTAL=0 cargo test -p mnemonic-mcp --features test-support --test integration_a2a_mcp_tools -- --include-ignored --nocapture
```

Recorded runner output: `/private/tmp/mnemonik-task5-parent-dedup-drills.log` — 11 passed, 0 failed, 0 ignored.
The temporary log is a local audit aid; the source tests and command are the reproducible evidence.

- `sdk_migrated_sealed_stream_recipient_continues_through_independent_operator` signs a checkpoint and storage manifest, then migrates three original sealed envelopes.
  It deletes receipts, destroys the original operator and source, and starts a fresh SDK process with the recipient identity.
  Destination-only restore compares every original envelope and plaintext, verifies grants, rejects outsider opening, and recovers a completed multi-chunk stream.
  A different operator accepts the recipient's continuation through the migrated parent locator, without restored SQL history.
  This integrates the checkpoint/exact-byte assertions and sealed operator-switch evidence missing from the baseline A-8/A-12 mapping.
- `two_operator_uploads_have_distinct_locators_and_sdk_verified_dedup` uploads identical bytes through two different operator identities.
  Both uploads preserve the artifact identity and original bytes, produce different locators, and import as one verified SDK artifact.
  This supplies the missing task 18/task 20 operator-rotation fixture.
- `invalid_external_parents_fail_before_upload_or_payment` tests wrong signed hash, wrong context, ineligible author, corrupt signature, outage, and missing locator.
  It asserts distinct error messages and zero upload, payment calls, delivery operations, paid operations and A2A receipts.
  This covers the mapped adversarial parent rejection assertions under A-9.
  The HTTP prevalidation correction now preserves retriable `ParentUnavailable` as HTTP 503 / JSON-RPC `-32011`; invalid or malformed/missing parent hints remain HTTP 400 / `-32602`. The complete binary passed again: 11 tests, zero ignored, including assertions that rejected requests perform no upload or financial operation.

The run also repeats the original SQL-loss drill and receipt-loss continuation checks.
The earlier explicit receipts-present/absent parity assertion, synthetic graph-guard coverage, and native parent deadline/size coverage still need separate mapping.
Live enumeration, fetch and lag acceptance under A-13 remains separate from these passing local drills.
Tasks 18–21 and wider release gates are not closed by this evidence update.

Final follow-up implementation: `98d18d0`; all 11 integrated tests passed after retryable-error correction. The [release-evidence record](../protocol-product/release-evidence.md) contains dated live inventory observations and the remaining positive-fixture/lag gate.
