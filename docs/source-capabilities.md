# Source capability matrix

Reviewed against `26a9550` (PR #268), 2026-10-02.
“Supported” below means implemented in this source revision with the stated scope.
It does not mean published, deployed, or accepted against live providers.
The [implementation evidence](../work/protocol-product/implementation-evidence.md) and [A2A acceptance record](../work/sealed-memories/a2a-acceptance-reconciliation.md) distinguish those results.

## Artifacts and client operations

| Artifact or operation | Preparation and delivery | Recovery and limits |
|---|---|---|
| Public general memory (`MEMORY_V1`) | SDK `preparePublicMemory` signs locally. Shared ingestion requires signed public visibility and explicit public consent | Native `recover_memory` verifies original envelope and pinned author. Content is plaintext at storage |
| Sealed general memory (`SEALED_V1`) | SDK `sealMemory` encrypts and signs locally. `store` is a session cache; `anchor` delivers original bytes | Native verified recovery supports canonical CBOR and the existing SDK inner JSON. Keys are required. SDK `openMemory` decrypts; it does not verify the signed envelope |
| Signed A2A task, message and artifact | SDK retains signed envelope in its configured index before HTTP submission. Local mode stays client-side; hosted local mode is rejected | Verified discovery/import checks identity, author, context and parents. Recovery reports gaps and forks; it cannot discover a provably newest head |
| Sealed A2A with recipient grants | Grants are bound into the signed artifact. Recipient cards and signing keys must be trusted independently | Author and recipient opening, completed encrypted streams, destination-only restore and independent operator continuation have local integration coverage |
| Separate general-memory grants | Existing cryptographic formats and grant reads remain | New hosted grant publication is retired. Shared ingestion does not accept independent grant artifacts; A2A embedded grants remain supported |
| Recovery checkpoint and backup | Versioned checkpoint signed by an independently pinned owner; encrypted backup takes explicit identity/key inputs | Trust must not come from the downloaded backup alone. Missing separately managed encryption keys can prevent recovery |
| Locator manifest | Owner-signed, bound to a verified checkpoint and exact-envelope digests | Migration API supports A2A, embedded grants and completed streams. General memory and live SSE stream migration are unsupported |

Source: [SDK client](../packages/sdk/src/client.ts), [native recovery](../core/src/rebuild.rs), [A2A recovery](../packages/sdk/src/a2a-recovery.ts), [checkpoints](../packages/sdk/src/checkpoint.ts), [backups](../packages/sdk/src/backup.ts), [migration](../packages/sdk/src/storage-portability.ts).

General memory identifiers and parent identifiers are not necessarily content hashes.
Verified memory graph recovery requires independent author and exact-envelope digest pins for heads and ancestors.
A2A parent identifiers bind signed content hashes; external locators are routing hints, not authority.
Neither format proves global completeness or truth of content.

## Storage and discovery

| Backend or source | Implemented capability | Explicit boundary |
|---|---|---|
| Caller-owned local storage | CLI persists sealed ciphertext; SDK accepts a caller-owned A2A index. Checkpoint and backup APIs return data for caller persistence | SDK default caches are session-only. Local SQL/index loss requires retained bytes, trusted checkpoints and keys |
| Arweave delivery | Operator uploads original signed bytes through ArDrive Turbo and checks exact fetched bytes | Availability is observed at read-back time. No unconditional retention, indexing, or timestamp guarantee follows |
| `ArweaveStorageAdapter` | Bounded `ar://` fetch through configured origin | Adapter upload is unsupported; use ingestion. Discovery is a separate API |
| `HttpObjectStorageAdapter` | `blob://<SHA256>` fetch and immutable PUT with exact read-back | One configured origin; server must enforce its retention/access policy. No deployed object service is supplied |
| Arweave discovery | Arweave GraphQL queries, cursors, known-locator fallback and structured source diagnostics | Indexes are untrusted discovery hints. Lag, omission and failure are reported; exhaustion is not completeness |
| Native parent resolution | `ar://`; optional configured blob origin via `MNEMONIC_PARENT_BLOB_ORIGIN` | Blob origin must be configured on the receiving operator. Parent authenticity is checked after bounded fetch |
| Historical Solana memos | Legacy verification and discovery | New shared ingestion does not require a new Solana memo |
| IPFS, Filecoin, arbitrary cloud providers | No supported adapter in this matrix | Design examples are not production integrations |

Both SDK storage adapters enforce 1 MiB objects, configured origins, no redirects and a 10-second deadline.
Use standalone `restoreMigratedA2A` for migrated blob locators.
Generic `importA2AAttestation` and direct checkpoint restore still use the Arweave gateway path.
Source: [adapter contracts](../packages/sdk/src/storage.ts), [native parent resolver](../core/src/arweave/mod.rs), [discovery implementation](../packages/sdk/src/discovery.ts), [storage guide](./storage-portability.md).

## Privacy and operational state

Client-prepared sealed requests contain ciphertext, never plaintext, recall keys or decryption keys.
Storage services still see envelopes, sizes, public fields, locator access and timing.
Legacy `signMemory` and `/api/sign-callback` prepare content on the server before client signing; they are not client-private.
Explicit HTTP local writes are rejected. Plain local stdio writes and signed client-local sealed artifacts are different contracts.

Hosted sealed storage, recall-session creation and new general-grant publication return HTTP 410.
SDK sealed recall requires a caller-supplied local embedder and never falls back to hosted embedding.
CLI general sealed recall is unsupported.

New shared ingestion stores delivery/financial metadata rather than artifact payloads.
Historical paid staging can retain old payloads until identical client resubmission drains them.
Thus an upgraded database is not automatically free of historical artifact bytes.
Legacy public indexing also remains separate from the private prepared-artifact path.

The shared paid coordinator supports the Universal Paywall exact rail; other rails fail closed there.
Delivery status and payment status are independent. Settled payment can coexist with failed delivery and a pending operator remedy.
Retry guarantees require durable financial records and provider idempotency; record loss requires reconciliation.
No automatic refund, universal price, or free-quota entitlement is promised here.
See [client preparation](./client-prepared-memory.md) and [payment retries](./artifact-payment-retries.md).

## Evidence and release boundary

Local drills use real SDK/WASM cryptography, HTTP dispatch, independent operator identities and mocked external services.
They cover loss of operational SQL, exact-byte migration, recipient opening, completed streams and continuation after source shutdown.
Payment crash tests use accepted mock receipt fixtures; they do not execute live settlement.
Live index lag, production retention, package contents and deployed configuration require separate recorded acceptance.
A successful health response alone establishes none of these capabilities.

The English papers link this matrix as their current implementation boundary.
Translated papers and older architectural proposals can describe earlier behavior; they are not a substitute for this source contract.
