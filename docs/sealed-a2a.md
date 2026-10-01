# Client-signed sealed A2A

The SDK creates the ciphertext, author wrap, recipient grants and every COSE
signature locally. HTTP MCP accepts only authenticated client-signed writes.
The server verifies signatures and bindings, then stores the original bytes in
SQLite. It never opens the ciphertext or signs for a remote identity.

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
| `mnemonic_attest_a2a` | `kind`, `context_id`, `signed` (hex COSE), optional `sealed` (default false), `prev_id` | `attestation_id`, `blake3`, `cose_envelope_hex`, `sealed`, `sealed_payload` |
| `mnemonic_recall_a2a` | `context_id`, optional `kind`, `limit` (1–1000), `sealed` | `attestations[]` with actual kind/time/context/parent/signer/hash, original COSE, public payload, sealed DataPart and optional stream |

The COSE payload is JCS of `mnemonic.a2a.signed.v1`: `kind`, `context_id`,
`prev_id`, `created_at`, `sealed`, `payload`, and optional `stream`. Every tool
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

Sealed storage has empty `content`, `privacy=sealed`, no embeddings, and retains
original signed bytes plus grant indexes atomically. Replay returns the same
ID. `prev_id` must exist in the same context; forks are allowed. Recall applies
author/active-grant, kind and sealed filters before LIMIT. Anonymous and
unrelated callers receive no rows. Legacy server-signed A2A rows are not
migrated into this signed-binding index. Stdio also requires a client-signed binding; no transport signs A2A
artifacts inside MCP.

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
card substitution and sealed stream mutations. SDK real-WASM tests open native
fixtures and reproduce exact signed bytes from their canonical payloads. Fresh
sealing uses randomness and is not expected to reproduce the same ciphertext.
The HTTP test runs the actual SDK/WASM through MCP, SQLite, recipient recall
and local open for messages/artifacts with and without chunks.

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
