# Local research handoff demo

This repeatable preparation fixture uses the real SDK, WebAssembly, signatures,
encryption, grants and HTTP operator handlers. Storage, discovery and both
operators run on loopback. It makes no live-provider retention or deployment
claim. Task 8 remains in progress while live task 5 prerequisites remain open.
The command below executes the local failure matrix.

## Run

Run from the repository root with Rust, Node 20+ and the SDK dependencies and
built SDK/WASM artifacts available. The test deliberately fails if those
artifacts are absent. Build them separately before starting concurrent native
Cargo work; the SDK build script recreates the shared WASM directories.

```sh
node scripts/run-protocol-demo.mjs
```

This runs the handoff, cryptographic/discovery injections and separate financial
fault tests sequentially. After every required assertion passes it combines their
observations into `target/protocol-demo/report.json` and `report.html`.
Open that HTML file in your browser. The failure view includes separate payment,
delivery and receipt-persistence observations. The handoff itself is unpaid;
financial tests seed accepted mock-provider receipts and do not execute settlement.

The runner removes older combined JSON and HTML before compiling or testing.
A failed run leaves `run-status.json` marked failed, with no combined success
report. Each attempt retains its component reports in a fresh `run-*` directory
for diagnosis. Those partial reports do not establish an overall pass.
The viewer displays recorded evidence; it does not contact services or trigger
injections. Fault controls run in the command-line harness.

Reports contain public author/operator keys, artifact identifiers, hashes,
locators and observed checks. They exclude private keys, JWTs, passphrases,
identity backups, original envelopes and payload plaintext. Local service
listeners stop when their tests finish.

To render an archived report without re-running its assertions:

```sh
node scripts/render-protocol-demo.mjs \
  target/protocol-demo/report.json target/protocol-demo/report.html
```

This only renders a historical snapshot. It does not independently verify the
original signatures or establish a new successful drill.

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

The combined runner covers the ten injections in the
[scenario specification](../work/protocol-product/proof-of-concept.md): changed
signed bytes, ciphertext tampering after public hash repair, forged discovery
tags, outsider opening, a missing parent, discovery outage, index lag with known
locators, a valid fork, receipt write failure and settled-delivery failure.

The ciphertext test deliberately uses the synthetic author's signing key to
re-sign repaired public metadata: public verification succeeds before real AEAD
decryption rejects the changed ciphertext. This does not demonstrate a signature
forgery. Discovery controls use synthetic provider responses with real verification
and graph recovery. Payment controls exercise durable states using seeded
provider receipts; they do not prove live settlement, refunds or provider behavior.

The integration binary also separately covers these related controls:

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

The local matrix does not close the live-provider acceptance gates.
Signed research can still be false. Grant eligibility does not establish general
project authority. Mock discovery does not prove live-provider coverage, and
successful delivery does not promise indefinite availability. See the
[acceptance plan](../work/protocol-product/acceptance.md) for remaining release gates.
