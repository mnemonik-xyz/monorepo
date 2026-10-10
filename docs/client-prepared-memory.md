# Client-prepared memory ingestion

This is the implementation boundary for product task 2. It is not a production availability or paid retry
claim. External delivery tests use a mocked Arweave gateway; encryption and signing use the real
WASM/native implementation.

## Prepare, retain, deliver

`MnemonicClient.sealMemory(content, {mode: "store"})` encrypts and signs on the client without any HTTP
request. It returns `memoryHash`, `outerCbor`, and `signedBytes`. The client keeps a session-only cache; the
caller must persist returned bytes and back up its identity for recovery after restart.
`openMemory(outerCbor)` decrypts locally; it is not a signature-verification API. Independent recovery must
verify the retained signed envelope and its author before trusting plaintext (see native `recover_memory`).
The existing sealed inner JSON contract and SEALED_V1 signed field order are preserved.

`sealMemory(content, {mode: "anchor"})` prepares the same private artifact and submits its original signed
bytes. Plaintext, decryption keys, and recall keys are never included in that request. It independently
fetches the resulting locator through the configured gateway and compares exact bytes. The gateway setting is
currently `a2aGatewayUrl`, shared with A2A recovery. A successful fetch establishes availability at that
observation time only.

After failed delivery, use `preparedSealedMemories()` to retrieve original bytes from the session cache and
`ingestPreparedMemory(signedBytes)` to resubmit. Do not call `sealMemory` again for a retry: encryption uses
fresh randomness, producing a different artifact. Persist prepared bytes before process exit.
`preparePublicMemory(content, {tags})` constructs and signs MEMORY_V1 locally; submitting it requires
`ingestPreparedMemory(signedBytes, {publicConsent: true})`. Public artifacts are plaintext at external
storage.

## HTTP contract

`POST /api/ingest-artifact` requires an authenticated JWT and `Content-Type: application/cbor`. Its body is
the complete binary COSE_Sign1 envelope, with a maximum of 1 MiB. `x-mnemonic-mode` must be absent or
`anchored`. Public MEMORY_V1 requires signed `visibility: "public"` and `x-mnemonic-public-consent: true`.
SEALED_V1 requires the authenticated author's `did:sol:` producer, supported encryption fields and valid
signature. Unknown kinds and malformed or mismatched authors fail before upload or quota consumption.
`/api/anchor-sealed` is a compatibility adapter to this same coordinator.

The delivery implementation uploads original bytes unchanged, fetches through the configured bounded gateway
and compares the exact envelope. The same exact-byte delivery primitive serves A2A. Memory receipts store only
author, digest, content hash, kind, byte count, locator and observation time in `artifact_receipts`. A SQL
receipt failure after verified external delivery returns `delivery_status: "verified"`, `receipt_persisted:
false`, and a persistence diagnostic. A2A similarly reports receipt persistence separately.

The payment coordinator is owned by task 3. Callers may supply `operationId` and `paymentHeaders` to
`ingestPreparedMemory`; retain the same operation and original bytes across wallet approval, payment and
retry. A rejected request throws `ServerError` with HTTP `status` and a structured `cause.response` containing
the challenge or retry state, plus `cause.paymentRequired` for the payment-required header. Free-quota
admission uses existing account/IP/global limits. A revalidated artifact already present as exact bytes does
not consume another free allocation. Financial acceptance and supported rails must follow task 3 validation;
task 2 alone does not establish them.

## Migration boundaries

HTTP A2A ingestion uses the same durable operation coordinator after signature, binding and external-parent
validation. The original MCP argument/result shapes are preserved with operation and payment state added.
Unsupported payment rails fail closed before payment or upload; supported rail evidence is recorded in task 3.


- `signMemory` and deferred `/api/sign-callback` remain a **legacy server-prepared** flow. The server receives plaintext and constructs the signing bundle. Local signatures do not make this flow client-private. Use `sealMemory` for private client preparation or `preparePublicMemory` for explicit public publication.
- Legacy inline/callback delivery now checks exact original bytes and expected signer without a SQL row existence requirement. Receipt persistence is reported separately. Legacy failure demotion and paid staging are separate migration concerns.
- Hosted `/api/store-sealed` returns 410. `mode: "store"` in the SDK now means the client session cache, not hosted durable storage.
- Hosted recall-session creation returns 410 and HTTP recall never decrypts sealed contents. Existing session cleanup remains available; restore and decrypt locally. SDK `recallSealed` ranks its local cache with an explicitly supplied local embedder; it never falls back to hosted embedding or index calls.
- New hosted `/api/grants` writes return 410. SDK `share` reports the migration explicitly before making a request. General memory grant publication is currently unsupported by the new ingestion coordinator. Retain signed grants client-side or use the existing sealed A2A recipient-grant path. Existing grant reads and withdrawal remain available; no existing rows are deleted by this change.
- SDK recall sends `limit`, preserves server evidence, and understands `total_attestations` and `created_at`. Evidence is not a completeness proof.

A2A SDK writes retain their original signed envelope in the configured client index before HTTP submission.
After a challenge or network failure, use `retryA2ADelivery(attestationId, {operationId, paymentHeaders,
prevLocator})`; it reuses those exact bytes and independently verifies the delivered artifact. The default
index is session-only; configure durable storage for process restarts.

## Validation

Targeted Rust tests: `cargo test -p mnemonic-mcp --features test-support --test signed_ingestion --test
sealed_routes --test integration_a2a_mcp_tools`.

Build real SDK/WASM once, then explicitly run `cargo test -p mnemonic-mcp --features test-support --test
signed_ingestion sdk_wasm_real_http_private_ingestion -- --ignored --nocapture`. This uses real HTTP dispatch
and real encryption/signatures with a mocked external gateway. It inspects hosted tables for absence of
memory/grant/vector payloads, independently fetches exact bytes, and opens from a fresh offline client. It is
not a live provider drill.

Targeted SDK tests: `npm test -w @mnemonik-xyz/sdk -- --run test/sealed.test.ts test/client.test.ts
test/integration/recall-verify.test.ts`.
