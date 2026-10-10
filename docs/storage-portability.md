# Storage migration

Status: available in source for signed A2A artifacts. Live backend migration and published-package drills remain separate release gates.

Migration copies original signed bytes. It preserves artifact identifiers, recipient grants, completed sealed streams and signed parent references.
The client signs a routing manifest after destination copies pass verification.
The manifest does not replace any artifact signature.

## Supported adapter contracts

| Adapter | Fetch | Upload | Discovery | Retention and consistency |
|---|---|---|---|---|
| `ArweaveStorageAdapter` | `ar://<43-character-id>` | Explicitly unsupported; use paid ingestion | Separate discovery interface | External Arweave/Irys policy; gateway availability and index lag remain dependencies |
| `HttpObjectStorageAdapter` | `blob://<64-lowercase-hex-SHA256>` | Immutable `PUT /objects/<digest>`, then exact read-back | Unsupported | Configured object server policy; no permanence claim |

Both adapters limit each object to 1 MiB and use a configured HTTP(S) origin.
Locators cannot select hosts, credentials, redirects or arbitrary paths.
Requests omit browser credentials and use a 10-second deadline.
The object adapter verifies SHA-256 after every fetch.
Servers see original bytes, object sizes, locator requests and timing.
Sealed content remains encrypted; public artifacts remain public.

The object server must enforce immutable writes and its own access and retention policies.
The SDK sends `If-None-Match: *` and validates existing objects after a precondition response.
No storage service is deployed by this implementation.
The Arweave adapter does not bypass ingestion payment or quota gates.

## Authenticated locator manifests

Version 1 signs restricted canonical JSON after `mnemonic.locator-manifest.v1` and one newline.
Its fields are `version`, `checkpointSha256`, `artifactKind`, `scope`, `createdAt` and `entries`.
Each entry binds an artifact identifier, author, exact-envelope SHA-256 and one or more locators.
The wrapper carries `manifest`, owner `signer` and a lowercase hexadecimal Ed25519 `signature`.

`checkpointSha256` hashes the domain-separated checkpoint bytes from `checkpointBytes`.
The caller independently pins the checkpoint owner and context.
Manifest verification checks that binding, signature and trusted authors before fetching artifacts.
Every fetched artifact also passes original signature, author, identity, context and exact-byte digest verification.
A manifest timestamp does not establish freshness or continued availability.

Limits are 1,000 artifacts, eight locators per artifact, a 1 MiB manifest and 64 MiB per migration or restore.
Version 1 supports A2A artifacts, including embedded grants and completed sealed streams.
General memory migration, independent grant artifacts and actual SSE event-stream migration are not supported by this API.

## Migration and restoration

```ts
const source = new ArweaveStorageAdapter(configuredGatewayOrigin);
const destination = new HttpObjectStorageAdapter(configuredObjectOrigin);
const manifest = await migrateA2AStorage(
  signedCheckpoint, pinnedOwner, contextId, [source], destination, ownerSigner,
);
// Back up manifest together with the independently authenticated checkpoint.
// Preserve source copies until destination restoration passes.

const report = await restoreMigratedA2A(
  manifest, signedCheckpoint, pinnedOwner, contextId, [destination], freshLocalIndex,
);
```

Migration verifies all checkpoint artifacts and ancestry before its first destination write.
Missing parents or absent pinned heads stop migration.
Each upload receives the original bytes and requires an exact read-back match.
The function never deletes source copies or accesses decryption keys.
A failed migration can leave verified destination objects without a completed manifest.
Retries reuse their content-addressed object names.

Restoration requires no original operator, index or source backend.
It tries authenticated locator alternatives until an exact verified copy succeeds.
It reports missing ancestry and preserves valid partial results.
Only ancestry reaching caller-pinned heads can set `completeToHeads`.
A fork is retained; it does not establish a newest head or global completeness.

Use `signLocatorManifest` to authenticate reviewed locator alternatives.
Signing a manifest alone makes no availability claim.
`restoreMigratedA2A` copies verified rows into an optional caller-owned index.
Configure the SDK with that index for later local or hosted continuation.
Use this standalone manifest restore for migrated `blob://` locators.
Generic `importA2AAttestation` and direct checkpoint restore still use the Arweave gateway path.
Recipient opening still requires the recipient's original decryption keys.

## Gateway read failover

This behavior is available now. Item reads try the configured primary gateway first.
Then they try each origin in `ARWEAVE_FALLBACK_GATEWAYS`, in order.
The default list is `https://arweave.net`, `https://ar-io.dev` and `https://turbo-gateway.com`.
An empty value disables failover. A local gateway gets no fallbacks.

Each gateway gets up to 3 attempts for connection errors, HTTP 429, HTTP 5xx and empty bodies.
The delay between attempts grows linearly. HTTP 404 and other client errors move to the next gateway at once.
A read reports "not found" only when every gateway returns HTTP 404.
Otherwise the error names each gateway and its failure reason.
A2A reads keep their 1 MiB limit and reject redirects on every gateway.
Callers still verify signatures and digests; a gateway is not a trust anchor.

## Independent operator continuation

An operator may configure `MNEMONIC_PARENT_BLOB_ORIGIN` with one trusted HTTP(S) origin.
Its parent resolver accepts `blob://<SHA256>` and fetches `/objects/<SHA256>` from that origin.
Without that configuration, blob-parent continuation is explicitly unsupported.
The resolver limits responses, rejects redirects and checks the exact SHA-256 before parent-signature validation.
Existing `ar://` parent resolution remains unchanged.

The SDK keeps the original signed `prev_id` and sends the migrated locator as `prev_locator`.
The independent operator needs its own ordinary upload and payment configuration.
It does not need the original operator's receipt database to validate the parent.
Operator financial-state loss remains an incident; migration does not authorize replaying settled payments.

## Evidence and limits

SDK tests use real WebAssembly signatures, grants, sealing and completed-stream verification.
The source and object services are separate mocks.
The tests disable source access, then restore through a fresh destination adapter and local index.
They open content as the recipient, reject an outsider and continue the original parent chain locally.
Additional tests reject missing ancestry before uploads, altered manifests, wrong owners, changed bytes and arbitrary origins.
Native tests exercise configured-origin parent fetches and digest validation.
MCP continuation tests provide separate evidence for a fresh operator with empty receipts.
These tests do not prove live provider availability, permanent retention or production migration readiness.
