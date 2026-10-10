# Mnemonic: Verifiable Memory for AI Agents

**Whitepaper** · Version 0.6 · 2 October 2026
**Technical details:** [Yellow paper](./YELLOWPAPER.md)
**Current source boundary:** [Capability matrix](./source-capabilities.md)
**Earlier Russian edition:** [WHITEPAPER_RU.md](./WHITEPAPER_RU.md)

This edition describes source capabilities at `26a9550`, not a released-package or live-service acceptance claim.
The Russian edition may describe earlier behavior.

## 1. Summary

An agent can keep useful context beyond one conversation, runtime, or service.
Mnemonic represents that context as signed artifacts that clients can retain, verify, and recover independently.
For private memory, the client encrypts and signs before requesting external storage.

The goal has three parts:

1. **Keep control.** Retain your identity, original artifacts, and recovery backups.
2. **Check provenance.** Verify an artifact against an independently trusted author's public key.
3. **Leave a service.** Recover supported history and continue through another operator without its old database.

The CLI, TypeScript SDK, and MCP service expose different parts of this workflow.
Their exact capabilities are listed in the [source matrix](./source-capabilities.md).

## 2. The problem

Saved context can depend on a single application, its database, or its account system.
Copying text alone loses evidence of authorship and links to earlier records.
Encrypted storage adds another dependency: the keys needed to open it.

Mnemonic separates signed memory from the services that deliver, discover, or index it.
Recovery still needs accessible artifact bytes, trusted authors and checkpoints, and any required decryption keys.
It restores recorded context, not a running process, tool credentials, or unfinished actions.

## 3. What a memory is

A record can contain content, tags, author metadata, parent references, and an optional embedding for search.
A signature authenticates the signed bytes under the author's key.
Encryption restricts who can open the content.
Embeddings help search; they do not establish whether a statement is true.

General memory and A2A records use different identifiers and parent contracts.
General memory identifiers alone do not authenticate an entire history.
Its verified recovery uses pinned authors and exact-envelope digests for heads and ancestors.
A2A parent identifiers bind content hashes.

## 4. How a private memory is saved

The current client-prepared flow is:

1. Construct the memory on the client.
2. Encrypt its content and sign the sealed envelope locally.
3. Retain the original signed bytes in client-controlled storage.
4. If requested, submit those exact bytes for external delivery.
5. Fetch the resulting location and check that it returns the same bytes.

The operator receives ciphertext and public metadata, not plaintext or decryption keys.
Read-back proves availability at that observation time; it does not guarantee future retention.

The CLI persists its sealed artifact locally before delivery.
The SDK default cache lasts only for its session; applications must supply durable persistence.
Local sealed signing requires an identity key.
It must not be confused with the older unsigned plain stdio-local path.

## 5. Choosing storage and visibility

| Choice | Current source behavior | Boundary |
|---|---|---|
| Local sealed memory | CLI stores original ciphertext; SDK can prepare without HTTP | Retain keys and artifacts. SDK session cache alone is not a backup |
| Externally delivered sealed memory | Original encrypted signed bytes go to Arweave through ArDrive Turbo | Metadata remains visible; availability depends on storage |
| Public memory | Explicit public consent permits plaintext publication | Anyone who obtains the bytes can read them |

The CLI defaults to sealed local writes.
`sign --anchor` requests sealed external delivery.
`sign --public --anchor` explicitly uses the legacy public plaintext path.
The SDK also supports client-prepared public artifacts with explicit consent.

These are user choices, not one universal mode enumeration across all APIs.
Explicit hosted local writes are rejected.

## 6. External delivery and time

Current shared ingestion uploads original signed bytes and verifies exact read-back.
The operator uploads through ArDrive Turbo, which bundles data items into Arweave transactions.
An upload counts as permanent only when an Arweave gateway reports it in a block.
It does not require a new Solana memo.
Historical Solana memo verification and discovery remain available.

A signature alone does not establish a trusted date.
An artifact's own timestamp is an author claim.
Any independent timestamp requires validation of that separate evidence.

Public storage does not give the author control over every copy.
Encryption does not hide all metadata or prevent an authorized reader from retaining plaintext.

## 7. Who signs and who sees content

The prepared-artifact client signs with its own identity.
An operator's transport signature does not replace the artifact author's signature.
The owner's local stdio identity can sign inline; remote users must provide their own signatures.

Legacy `signMemory` and deferred signing send plaintext to the operator for preparation before client signing.
A client signature does not make that legacy flow private from the operator.
Hosted recall-session creation is retired; new private recall and opening happen locally.

The CLI's standalone initialization creates a file-backed identity with restricted permissions.
Existing keychain-only identities cannot use the current offline sealed CLI write.
See the [CLI guide](../packages/cli/README.md) for identity and backup behavior.

## 8. Sharing a memory

Sealed artifacts encrypt content with a random content key.
Recipient wraps allow approved keys to recover that key.
A recipient who receives plaintext or a usable key can keep it; later revocation cannot undo that access.

Sealed A2A supports signed recipient grants and completed encrypted streams.
The author signs the plaintext and its recipients before encryption, and signs the ciphertext after it.
Reader identity and encryption-key bindings must be verified against independently trusted signing keys.

General-memory grant formats and existing grant reads remain, but new hosted grant publication is retired.
The shared ingestion coordinator does not accept independent grant artifacts.
Bearer-link publication and scoped revocable capability tokens are not supported promises of the current prepared-memory path.

## 9. Verification and recovery

Verification checks the retained envelope and the expected author before trusting its contents.
It can run locally without the original operator, account, or protocol transaction.
Decryption alone is not signature verification.

An owner-signed checkpoint records trusted authors, expected heads and locations.
The caller pins the owner's public key independently of the downloaded checkpoint.
An encrypted backup can carry the checkpoint and explicitly supplied key material.

Recovery reports unavailable sources, invalid artifacts, missing ancestors and competing branches.
Completion covers only ancestry to independently trusted expected heads.
An index cannot prove that no newer, undisclosed branch exists.
See [recovery checkpoints](./recovery-checkpoints.md).

## 10. Safe use of shared memories

A valid signed memory can contain false information or malicious instructions.
Consumers must apply their own trust and content-handling policies.
Proposed framing markers and capability policies do not establish a model's compliance.
They are not implemented security guarantees of current recall.

## 11. Implemented properties

| Property | Conditions |
|---|---|
| Detect altered signed bytes | Verify the signature and expected author's key |
| Recover without old operator SQL | Retain trusted recovery inputs, usable keys and accessible original bytes |
| Preserve provenance during migration | Copy original envelopes and verify authenticated locator manifests |
| Continue A2A through another operator | Verify external parents; configure the destination operator's supported locator resolver |
| Run verification locally | Possess the bytes, trust pins and compatible verifier |

Local drills exercise these conditions with real cryptography and mocked external services.
They do not establish universal backend support or live-provider retention.

## 12. Explicit limits

- Signatures establish provenance and integrity, not truth or trusted time by themselves.
- Recovery cannot prove global completeness or reveal every hidden newer head.
- Losing required keys and their backups can make ciphertext unreadable.
- Migration currently supports A2A, embedded grants and completed streams, not all artifact kinds or live SSE streams.
- Neither signatures nor encryption force downstream agents to handle content safely.
- Restored memory is not complete execution-state recovery.

## 13. Costs and failed delivery

Local cryptographic verification requires no protocol payment; running clients and storage still uses resources.
Operators can charge for external services under their configured quotes and quota rules.
This paper promises no fixed price or free allocation.

The shared paid ingestion path supports the Universal Paywall exact rail.
Other rails fail closed there; legacy payment interfaces have separate migration boundaries.
Delivery and payment are independent outcomes: a settled payment can have failed or pending delivery.
Retries reuse original bytes and durable operation records.
Terminal delivery failure requires an operator remedy; it does not trigger an automatic refund.
See [payment and retry behavior](./artifact-payment-retries.md).

## 14. Relation to other standards

MCP connects agents to tools; Mnemonic exposes service operations through MCP.
A2A support records signed task, message and artifact history, including sealed content and externally verified parent links.
ERC-8004 integration, richer cognitive schemas and formal capability tokens remain design work.
They are not prerequisites for current local verification or recovery.

## 15. Current evidence

The source includes client preparation, exact-byte ingestion, verified recovery, checkpoints, backups and bounded A2A storage migration.
Tests cover receipt loss, source shutdown, recipient opening and continuation through an independent operator.
Storage and discovery services in those integration tests are mocks.
Payment fixtures do not establish live settlement.

Read the [capability matrix](./source-capabilities.md), [implementation evidence](../work/protocol-product/implementation-evidence.md), and [acceptance reconciliation](../work/sealed-memories/a2a-acceptance-reconciliation.md).
Published packages, deployed configuration, live discovery lag and release readiness require separate evidence.

## 16. Glossary

| Term | Meaning |
|---|---|
| Artifact | A typed record with its signed envelope |
| Checkpoint | Authenticated recovery expectations, including trusted heads and locations |
| Envelope | The signed bytes retained for independent verification |
| Grant | A cryptographic recipient binding that permits content-key recovery |
| Head | An expected tip of recorded history |
| Locator | A routing hint for fetching artifact bytes |
| Manifest | Authenticated mapping from artifacts to exact copies at storage locations |
| Operator | A service that delivers or discovers artifacts |
| Sealed | Encrypted for authorized keys, with signed outer metadata |
