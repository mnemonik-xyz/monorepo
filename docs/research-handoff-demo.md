# Local research handoff demo

This repeatable preparation fixture uses the real SDK, WebAssembly, signatures,
encryption, grants and HTTP operator handlers. Storage, discovery and both
operators run on loopback. It makes no live-provider retention or deployment
claim. Task 8 remains in progress while live task 5 prerequisites and the full
failure matrix remain open.

## Run

Run from the repository root with Rust, Node 20+ and the SDK dependencies and
built SDK/WASM artifacts available. The test deliberately fails if those
artifacts are absent. Build them separately before starting concurrent native
Cargo work; the SDK build script recreates the shared WASM directories.

```sh
MNEMONIC_DEMO_EVIDENCE_DIR="$PWD/target/protocol-demo" \
CARGO_INCREMENTAL=0 cargo test -p mnemonic-mcp --features test-support \
  --test integration_a2a_mcp_tools \
  sdk_migrated_sealed_stream_recipient_continues_through_independent_operator \
  -- --ignored --nocapture
```

After a successful run, `target/protocol-demo/report.json` contains a versioned
artifact timeline and observed checks. The test deletes an existing `report.json`
before starting the scenario, so a failed attempt cannot leave an older success
report at that path. Compilation failures occur before the test starts; check the
command exit status before viewing any report. The generated report contains public author/operator keys,
artifact identifiers, hashes, locators, counts and check results. It excludes
private keys, JWTs, passphrases, encrypted identity backups, original envelopes
and payload plaintext. Loopback locators describe that completed test run;
the listeners stop when the test ends.

To render the recorded evidence into a local viewer after the command succeeds:

```sh
node scripts/render-protocol-demo.mjs \
  target/protocol-demo/report.json target/protocol-demo/report.html
```

Open `target/protocol-demo/report.html` in your browser. The viewer displays the
recorded report; it does not contact the stopped services or run live fault
controls. Render it again after each successful demo. An HTML file left from a
previous run remains a historical snapshot, not evidence of the latest attempt.

## Story and evidence

| Artifact | Author | Relationship | Evidence |
|---|---|---|---|
| R | Collector A | Root | Signed, sealed research question and scope |
| S | Collector A | Parent R | Completed sealed source stream with a signed grant to B |
| V | Reviewer B | Parent S | B discovers and opens A's artifacts, then signs its own review |
| W | Restored B | Parent V | Fresh B restores R/S/V and delivers a continuation through O2 |

All research content is deterministic and synthetic. There is no model call.
B starts its first review with a separate empty index and discovers A's records
through the external test index. B signs the checkpoint to V, including both
expected authors, and authenticates the migration locator manifest.

The storage adapter copies original bytes into the destination and checks them
again. The test deletes O1's routing receipts, stops O1, drops its test database,
erases the source storage, and stops that listener. This uses an unpaid fixture;
it is not a procedure for deleting financial records in an operating service.

A second Node process restores B from an authenticated encrypted identity
backup. It uses the pinned checkpoint and destination manifest to recover R/S/V,
compares their exact original envelopes, checks both authors and ancestry, and
opens all three locally. The fresh process does not use source discovery or O1.
The backup passphrase stays in the test process environment and is not published.
This is restoration of B's identity, not provisioning an independently trusted C.

W is signed by restored B. O2 has a new operator key and an empty receipt database.
It verifies V through its configured external `blob://` parent resolver and
delivers W. O2 retains only W's new routing receipt; payload tables remain empty.
The report's recovered completeness is bounded to checkpoint head V. This demo
does not save a new recovery checkpoint to W.

## Failure controls and limits

The main demo executes source shutdown, O1 shutdown/receipt loss, and an outsider
attempt to open S. Only those executed controls appear in its JSON `failures`
array. The same integration binary separately covers the following controls:

| Test | Control |
|---|---|
| `invalid_external_parents_fail_before_upload_or_payment` | Wrong parent hash/context/signature, ineligible writer, absent parent and malformed/missing hints; no upload or payment side effects |
| `independent_operator_continues_from_migrated_parent_without_source` | Disabled destination resolver and corrupt destination parent bytes |
| `failed_delivery_is_pending_and_retry_uses_existing_remote_bytes` | External delivery failure and exact-byte retry |
| `two_operator_uploads_have_distinct_locators_and_sdk_verified_dedup` | Same signed artifact under two uploader keys, externally verified deduplication |

Run this related local evidence explicitly with:

```sh
CARGO_INCREMENTAL=0 cargo test -p mnemonic-mcp --features test-support \
  --test integration_a2a_mcp_tools -- --include-ignored --nocapture
```

These controls do not cover every injection in the
[scenario specification](../work/protocol-product/proof-of-concept.md).
Signed research can still be false. Grant eligibility does not establish general
project authority. Mock discovery does not prove live-provider coverage, and
successful delivery does not promise indefinite availability. See the
[acceptance plan](../work/protocol-product/acceptance.md) for remaining release gates.
