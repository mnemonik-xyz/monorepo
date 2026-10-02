# Task 5: integrated and live evidence

Baseline: merged `7dc5079` (PR #267). Observed on 2026-10-02.
This record extends the [implementation evidence](implementation-evidence.md), not the release or production support claims.

## Integrated local drill

The real SDK and WebAssembly run through HTTP against two independently keyed local MCP operators and separate storage services.
The drill signs three sealed artifacts, including a completed multichunk stream and recipient grants.
It signs a recovery checkpoint and locator manifest, copies and compares original envelopes, deletes the first operator's receipts, and stops its HTTP service and source storage.
A new Node process restores through destination storage alone, opens content as the recipient, rejects an outsider, and appends a recipient-signed child through the second operator.
The second operator verifies the migrated parent through its configured blob origin without the first operator's SQL or login.

Additional cases upload the same signed bytes through two actual operator keys and verify SDK deduplication despite different transport locators.
Negative parent cases cover wrong signed hash, context, signature and writer eligibility before upload or payment side effects.
See [A2A acceptance reconciliation](../sealed-memories/a2a-acceptance-reconciliation.md) for the per-task mapping.

```sh
CARGO_INCREMENTAL=0 cargo test -p mnemonic-mcp --features test-support \
  --test integration_a2a_mcp_tools -- --include-ignored --nocapture
```

Final result: 11 passed, zero ignored, including the error-classification correction.
An unavailable parent now returns HTTP 503 / JSON-RPC `-32011`; invalid or malformed/missing parent inputs return HTTP 400 / `-32602`.
Integrated tests, retry classification and live-probe implementation: `98d18d0`. Strict MCP library/binary Clippy also passed.
Storage and payment services are local mocks. This is not an independently deployed operator or live settlement drill.

## Live read-only observations

The reproducible [probe](../../scripts/protocol-live-evidence.mjs) sends no credentials, signs nothing and uploads nothing.
Its [raw observation record](evidence/live-discovery-2026-10-02.json) contains public metadata, timestamps and bounded scan results.

```sh
node scripts/protocol-live-evidence.mjs --samples 2 --interval-ms 1000
```

Observed window: `2026-10-02T12:14:32Z` through the recorded `finished_at`.
The command exited **2**, meaning no positive pinned fixture was established.

| Endpoint / filter | Two observations | Interpretation |
|---|---|---|
| `https://uploader.irys.xyz/graphql`, App-Name | 24 candidates, no next page | Public inventory was reachable. These are untrusted index hints, not verified Mnemonik artifacts |
| Same Irys endpoint, App-Name + A2A kind | Zero candidates | No positive A2A discovery/recovery evidence |
| `https://arweave.net/graphql`, App-Name | Zero candidates | Not an interchangeable positive index for these observations |
| Same Arweave endpoint, App-Name + A2A kind | Zero candidates | No positive A2A discovery/recovery evidence |

Twenty-one returned inventory IDs do not match the current `ar://` adapter's 43-character identifier shape; three match the shape only.
The probe counts other shapes without converting them into supported locators or claiming cryptographic validity.
There was no independently pinned A2A author, context, head and locator available for an exact verified fetch.
The earlier HTTP 403 observation is historical; today's successful responses do not erase it or establish sustained availability.

Two repeated index reads are an observation window, **not an ingestion-lag measurement**.
Signed artifact creation time is not an independently observed storage-submission time.
To measure lag, preserve the exact signed synthetic fixture and delivery/submission timestamp, then poll its independently pinned identity within a declared window.
Do not use an empty index or empty memo comparison to clear a positive recovery gate.
The legacy `enumerate` example now exits 3 (inconclusive) when its memo sample is empty. Running the actual example against local empty GraphQL/RPC services confirmed that result; it no longer prints a vacuous parity pass.

With an existing public synthetic fixture, the probe can also exercise the actual SDK discovery adapters and exact gateway verification:

```sh
node scripts/protocol-live-evidence.mjs \
  --author '<independently-pinned-public-key>' --context '<synthetic-context>' \
  --head 'a2a:<signed-binding-hash>' --locator 'ar://<item-id>' \
  --samples 6 --interval-ms 10000
```

Exit **0** means the pinned fixture was verified and found through at least one configured index in every sample.
It does not establish all providers, complete ancestry, recipient opening, submission lag or release acceptance.
Exit **2** means missing pins or unsuccessful positive observation; malformed invocation exits nonzero before requests.
Build the SDK first using the repository build instructions.

Separate public surface probes returned MCP health HTTP 200, anonymous `mnemonic_whoami` rejection for missing JWT, and HTTP 404 for the advertised agent-card path on the apex site.
A follow-up at `https://www.mnemonik.xyz/.well-known/agent.json` returned HTTP 200 and a valid Mnemonic service card. README/agent-guide discovery links now use that observed working host.
No authentication bypass or state-changing request was attempted.

## Deployment build findings

Build correction commit: `6301dea`. Two source build defects predate PR #267:

- [Webapp workflow failure](https://github.com/mnemonik-xyz/monorepo/actions/runs/37004456455): root lockfile omitted the conformance workspace. The minimal lock entries restore clean `npm ci`.
- [Container workflow failure](https://github.com/mnemonik-xyz/monorepo/actions/runs/37004456235): image build copied only two of four Cargo workspace members. Both Dockerfiles now include A2A members; the image workflow watches their paths.

Clean webapp compilation also exposed a nonexistent WASM `key_commitment` call.
The page now relies on the native `open_with_key` commitment check, decodes its byte result, and rejects absent crypto or malformed plaintext.
The pinned npm WASM package still lacks that opener. Build success does not enable sealed decryption in that deployed package; the page reports the unsupported capability.

Checks passed in an isolated source copy: clean `npm ci`, TypeScript/Vite/prerender build, 16 SealedView tests, and locked/offline Cargo metadata with all four copied workspace members.
The build left `VITE_MCP_BASE` unset, so live blog prerender was skipped.
No complete container build, image publication or deployment was performed.
The Cloudflare check exposes only a generic build failure; its authenticated dashboard logs were unavailable. Its precise cause remains unconfirmed.

## Remaining gate

A-8/A-9/A-12 have stronger local integrated evidence. Task 5 remains in progress until its existing A2A prerequisites and A-13 live evidence are satisfied.
The missing live input is an independently pinned synthetic A2A fixture plus supported staging/operator configuration and a recorded submission observation for lag.
Positive live fetch, recipient recovery and independent deployed-operator continuation must be recorded separately from this local drill.
The release package/install/rollback drill, customer evidence and task 14/#242/#233 closure remain separate.
