# Client-signed sealed A2A

The SDK creates the ciphertext, author wrap, recipient grants and every COSE
signature locally. HTTP MCP accepts only authenticated client-signed writes.
The server verifies signatures and bindings, then delivers original bytes to
Arweave/Irys. Hosted SQLite holds routing receipts only. The client signs artifacts;
the operator signs the ANS-104 transport upload. Decryption stays on the client.

```ts
const id = await sender.attestA2AMessage(message, contextId, {
  sealed: {
    recipients: [{ card: signedRecipientCard, trustedCardSigner: pinnedCardKey }],
    // Optional: seal a completed payload as a chunk chain.
    chunkSize: 16384,
  },
});
const rows = await recipient.recallA2AContext(contextId, { sealed: true });
const messageAgain = await recipient.openA2AAttestation(rows[0], trustedAuthorKey);
```

Bind a local keypair or keypair provider to both clients. `trustedCardSigner`
must come from trusted configuration, not from a key advertised by the card.
The complete card without `signatures` is JCS-canonicalized and verified using
detached EdDSA JWS. Exactly one `x-mnemonic` extension is allowed. Its Ed25519
identity maps to X25519 by default. An optional `enc_key` is standard base64 of
32 X25519 public-key bytes covered by the same JWS. A recipient with that
separate key supplies its 32-byte X25519 secret as the third argument to
`openA2AAttestation`; the signing identity still selects its grant.

`verifyA2AAttestation(coseEnvelopeHex, trustedAuthorKey)` verifies authorship,
existence of the signed ciphertext and the public chunk chain without a secret.
It does not prove that the sender encrypted useful or truthful content.

## MCP contract

| Tool | Inputs | Output |
|---|---|---|
| `mnemonic_attest_a2a` | `kind`, `context_id`, `signed` (hex COSE), optional `sealed` (default false), `prev_id`, `prev_locator` | `attestation_id`, `blake3`, `sealed`, `arweave_tx`, `locator`, delivery status and receipt persistence result |
| `mnemonic_recall_a2a` | `context_id`, optional `kind`, `limit` (1–1000), `sealed` | `receipts[]`; the SDK fetches and verifies original artifacts before returning attestations |

The COSE payload is JCS of the outer binding: `protocol`, `kind`, `context_id`,
`prev_id`, `created_at`, `sealed`, `payload`, and optional `stream`. Plain
envelopes use `mnemonic.a2a.signed.v1`. New sealed envelopes use
`mnemonic.a2a.signed.v2` (see [Sign-encrypt-sign](#sign-encrypt-sign)). Every tool
argument must agree with the signed binding. The JWT subject must equal the
signer. Invalid signatures and mismatched bindings fail before payment.
Writes remain paid on x402 deployments; recall remains free.

Sealed message/artifact carriers contain exactly one
`application/vnd.mnemonic.sealed+cbor` DataPart. `data.sealed` and each
`data.grants[]` are base64 **COSE-signed** SEALED_V1/GRANT_V1 bytes. All inner
signers must equal the binding signer. Grants bind the hash of the sealed CBOR,
recipient identity and wrapped key. Raw unsigned CBOR from low-level pack
helpers is insufficient for ingestion. Task objects support client signatures
but cannot be sealed by this API.

Hosted A2A writes store no artifact, ciphertext, grant blob or embedding in SQL.
Receipts may hold hashes, locators, author/reader identities and signed routing
metadata. Recall returns `{receipts:[...]}`; the SDK fetches original bytes and
checks them against receipt metadata. SQL receipt loss cannot invalidate parents:
non-root writes pass `prev_locator` alongside the existing signed `prev_id`.
The server verifies the fetched parent hash, context and author/named-reader link
eligibility. This link is not a general context write capability. Format-2 arlocal
uploads are explicitly unsupported for this ANS-104 A2A ingestion path.

### Recovery without MCP

```ts
await client.importA2AAttestation("ar://...", independentlyTrustedAuthor);
const report = await client.restoreA2AContext(contextId, {
  expectedAuthors: [independentlyTrustedAuthor],
  heads: [pinnedHeadId],
});
const localRows = await client.recallA2AContext(contextId, {mode: "local"});
```

Configure `a2aGatewayUrl`, `a2aIndexUrl`, and `a2aIndexFlavour` separately.
The external index is an unsigned discovery hint; signatures establish trust.
Requests carry no MCP JWT. Known-locator import works before index visibility.
Discovery retains verified forks and reports budgets, errors and missing parents.
Completeness is only ancestry to caller-pinned heads, never proof that every
artifact or newer head was indexed. The Irys provider remains an availability
dependency. Live production enumeration has not been demonstrated by this draft.

`mode: "local"` signs and stores on the agent's device without network. Supply
`a2aIndex` or `setA2AIndexStore` for persistence; the SDK default is session-only.
The CLI uses identity-scoped private JSON files and atomic replacement/locking.
A crash can leave a `.lock` file; remove it only after confirming no writer runs.
No automated migration of the rejected draft SQL artifacts is performed.

## Sign-encrypt-sign

Available now: sealed envelopes use sign-encrypt-sign. A ciphertext signature
alone does not show that the author signed the plaintext for a given reader.
The author therefore signs three layers:

1. **Inner signature.** The author signs JCS of `mnemonic.a2a.inner.v1` as
   COSE_Sign1. This binding holds `author`, sorted unique `recipients`,
   `kind`, `context_id`, `prev_id`, `created_at` and the plaintext `payload`.
2. **Encryption.** The client encrypts the inner COSE bytes with K. It wraps K
   for the author and for each recipient's X25519 key.
3. **Outer signatures.** The author signs the SEALED_V1 ciphertext, each
   GRANT_V1 and the `mnemonic.a2a.signed.v2` carrier binding.

After decryption, the reader checks these conditions:

- The inner signer is the expected author and the outer signer.
- `kind`, `context_id`, `prev_id` and `created_at` agree with the outer binding.
- The inner recipient list equals the set of signed grant readers.
- The reader is the author or a member of the inner recipient list.

A V2 envelope that holds unsigned plaintext fails. These checks stop two
attacks. A recipient cannot re-encrypt the author's signed plaintext to a third
party as its own message. A forwarder also cannot widen the reader set.

In Rust, `open_signed_a2a_full` returns the payload and the inner COSE bytes.
A reader can give the inner bytes to a third party. `verify_a2a_inner` then
proves that the author sent this plaintext to the named recipients. Showing the
inner bytes discloses the plaintext. SDK and WASM access to the inner bytes is
planned; `openA2AAttestation` returns the payload only.

Legacy `mnemonic.a2a.signed.v1` sealed envelopes still verify and open. They
carry no inner signature, so they give no plaintext-to-recipient proof.
`open_signed_a2a_full` returns `inner_signed: None` for them. The fixture
`sealed-a2a-v1-legacy.json` keeps V1 coverage.

## Sealed chunk chain

This implementation seals a **completed** payload with one K and a fresh
19-byte nonce prefix. Each nonce binds its 32-bit index and final flag. AEAD
AAD binds domain, sealed header hash, stream ID, index, final flag and previous
chunk hash. The author signature covers the entire manifest and final head.
The public chain must be dense and complete before any plaintext is returned;
client open then verifies every AEAD tag and the decrypted payload hash.
Reorder, omission, truncation, splice, changed header and ciphertext fail.
Recomputing public hashes cannot repair invalid AEAD tags.

This is not the per-SSE-event `A2A_STREAM_CHUNK_V1` transport requested in #61.
That adapter and its #61 hash-chain integration remain prerequisites for the
full streaming clause of task 14/#242. Existing low-level `seal_chunk` helpers
are unchanged: their index-only nonce contract does not provide the stream,
finality and header binding required here.

## Threat notes and completion

Public metadata includes identity, context, IDs, message role, time, lineage,
recipient identities, sizes and chunk count. Names, text, files and other A2A
fields are inside the ciphertext. Use opaque context IDs; encryption does not
hide traffic metadata. A recipient can retain plaintext or K forever;
withdrawal only stops future server recall. HPKE ephemeral wrapping is not
forward secrecy against later compromise of the recipient's long-term key.
Separate encryption keys and pinned card-signing keys require an application
rotation/trust policy. `did:mnemonic` key discovery (#74) is not implemented
by this patch; callers supply verified cards and trusted keys explicitly.

Fixtures contain public **test-only** seeds. Native tests cover sender/reader,
outsider, signed grant substitution, invalid wrap, distinct encryption keys,
card substitution, sealed stream mutations, recipient re-encryption of the
signed inner payload, a widened reader set and legacy V1 envelopes. SDK real-WASM tests open native
fixtures and reproduce exact signed bytes from their canonical payloads. Fresh
sealing uses randomness and is not expected to reproduce the same ciphertext.
The HTTP test uses actual SDK/WASM and a separate remote artifact/index service.
It deletes all MCP A2A receipts, disables MCP, restores a three-artifact sealed
chain as author/recipient, rejects outsider open, then restarts MCP with empty
receipts and extends the chain. The remote service is mocked; cryptography is real.

```sh
cargo test -p mnemonic-core --features a2a-experimental
cargo test -p mnemonic-mcp --features test-support --test integration_a2a_mcp_tools
npm run build --workspace @mnemonik-xyz/sdk
npm test --workspace @mnemonik-xyz/sdk
cargo test -p mnemonic-mcp --features test-support --test integration_a2a_mcp_tools sdk_http_sealed_end_to_end -- --ignored
```

Task 14 can close after this change is merged, required conformance runs pass,
and sealed chunks are connected to and tested in the actual #61 chain.
#242 additionally needs the stated recipient-discovery scope (#74), or an
explicitly agreed AgentCard-only V1 scope. #233 is an umbrella; this patch does
not complete its conformance publication, sidecar or use-case tasks.

## Draft implementation validation

SDK: 343 tests passed. CLI: 187 passed, 2 skipped (including concurrent durable
index writers). MCP: five targeted tests plus the explicitly enabled real HTTP/WASM
recovery test passed. TypeScript build passed after index locking. Strict production clippy (`--workspace --lib --bins -D warnings`) passes. The
workspace run passed 1,265 tests; sequential doctest rerun resolved concurrent
build crate-version mismatches. The live Irys query returned HTTP
403 in this environment; production index recovery remains unverified. Tasks 18–21 and task 14/#242 are not closed by
publishing this draft. Review against the merged recovery specification.

## Replaceable discovery sources

Available now in the SDK: pass `discoverySource` to `restoreA2AContext` to select an index adapter.
`IrysDiscoverySource` and `ArweaveDiscoverySource` use their respective GraphQL schemas.
Custom adapters implement `DiscoverySource` and return candidate locators with optional public metadata.
The client verifies original bytes against the caller's independently trusted authors.
An adapter cannot expand that trusted set.

```ts
const source = new IrysDiscoverySource('https://uploader.irys.xyz/graphql');
const report = await client.restoreA2AContext(contextId, {
  expectedAuthors: [trustedAuthor],
  heads: authenticatedHeads,
  discoverySource: source,
  maxPages: 10,
  maxCandidates: 1000,
});
```

`report.source` records source identity, status, page count, candidate count, and any continuation cursor.
Statuses distinguish exhausted scans, budgets, unavailable sources, malformed responses, cancellation, and disabled discovery.
Continuation checkpoints bind the context, authors, source identity, and supported backends.
Changing an endpoint or index flavour requires a fresh scan.
Previously verified local entries remain available during partial scans or source replacement.

Pass `discoverySource: false` and explicit `parentLocators` to recover known artifacts without an index.
The locator map can contain pinned heads and their required ancestors.
Each entry binds an artifact ID to its locator and independently trusted author.
A complete scan does not establish global completeness.
`completeToHeads` only establishes verified ancestry for the caller's pinned heads.

Built-in adapters omit credentials, reject redirects, and bound each page request and body read to 15 seconds.
Responses have a one-MiB size limit; pages contain at most 100 candidates.
Adapters expose only public routing metadata and never receive content keys or private recall data.
Custom adapter implementations must preserve these privacy and network limits.

Validation uses real signed fixtures with mocked providers.
It covers replacement, duplicates, omission, lag, forged hints, unavailable sources, cursor loops, and scan budgets.
These tests do not establish live index availability or production enumeration parity.


### Shared ingestion and migrated parents

HTTP `mnemonic_attest_a2a` validates signature, bindings and the external parent
before passing original signed bytes to the same durable operation coordinator
as memory ingestion. Retries use the same digest-bound operation; the earlier
bespoke settlement/nonce path is retired. Unsupported payment rails fail closed
before payment/upload. Supported rail and financial acceptance evidence belongs
to product task 3. The response retains A2A IDs/locators and adds operation and
payment state. External success remains distinct from SQL receipt persistence.

For migrated parents, operators can opt into `MNEMONIC_PARENT_BLOB_ORIGIN`.
`blob://<sha256>` locators resolve only under that configured origin's
`/objects/<digest>` endpoint, with bounded reads, no redirects, and digest/signature/
parent-link validation. The default is disabled. An independent-operator test
stops the old storage listener, clears original receipts, and continues against
copied exact parent bytes; this uses local mocked services, not production storage.
