# Client recovery and encrypted backups

Status: available in source. Published-package and live-provider release drills remain planned.

Recovery never proves that an index disclosed every artifact or newer head.
A complete result covers only ancestry to independently trusted heads.
Keep the owner's public key outside the backup file as a trust anchor.

## Signed checkpoints

`signRecoveryCheckpoint` signs canonical JSON with the existing Ed25519 signer.
The signed bytes start with `mnemonic.recovery-checkpoint.v1` and one newline.
The remaining bytes use the restricted JSON Canonicalization Scheme implementation in the SDK.
Object keys are sorted. Array order is preserved and signed.

Version 1 contains these fields:

| Field | Meaning |
|---|---|
| `version` | Exactly `1` |
| `artifactKind` | `memory` or `a2a` |
| `scope` | Caller-defined memory corpus or exact A2A context |
| `expectedAuthors` | Independently approved artifact authors |
| `heads` | Expected artifact identifiers; an empty list makes no completeness claim |
| `locators` | Artifact identifier, author and `ar://` locator bindings; memory entries also require `envelopeDigest` |
| `backendHints` | Descriptive provider hints; these do not authorize network destinations |
| `createdAt` | UTC time with milliseconds; this does not prove freshness |

Memory identifiers alone are not content addressed.
Every memory head requires an author and BLAKE3 digest of its exact signed envelope.
Unknown fields and versions fail validation. Checkpoints contain no private keys.
The signed wrapper contains `checkpoint`, `signer` and a lowercase hexadecimal signature.
`verifyRecoveryCheckpoint` requires the trusted owner, artifact kind and scope separately.
Never derive that trust anchor from the fetched checkpoint itself.

`checkpointA2ARestoreOptions` verifies a checkpoint before creating A2A recovery options.
It transfers trusted authors, heads and known parent locators.
The current A2A adapter requires one selected locator for each artifact identifier.
It rejects multiple locators instead of silently choosing one.
The checkpoint format preserves multiple locators for future migration workflows.

## Portable backup workflow

Explicitly pass an unlocked identity to `createRecoveryBackup`.
The function does not read a keychain or contact any service.
It encrypts the identity, signed checkpoint and optional separately managed X25519 keys.
Encryption uses Web Crypto AES-256-GCM with a fresh 12-byte nonce and a 16-byte salt.
PBKDF2-SHA256 derives the encryption key with 600,000 iterations.
The authenticated domain is `mnemonic.recovery-backup.v1`.
The passphrase must contain at least 12 characters; choose a strong independent passphrase.

```ts
const signed = await signRecoveryCheckpoint(checkpoint, new LocalSigner(identity));
const backup = await createRecoveryBackup(signed, identity, backupPassphrase);
// Save JSON.stringify(backup) to a user-controlled private backup destination.
// Store the passphrase and pinned owner public key separately.

const restored = await openRecoveryBackup(
  backup,
  backupPassphrase,
  independentlyPinnedOwner,
  { artifactKind: 'a2a', scope: contextId },
);
freshClient.setKeypair(restored.identity);
const options = await checkpointA2ARestoreOptions(
  restored.signedCheckpoint, independentlyPinnedOwner, contextId,
);
const report = await freshClient.restoreA2AContext(contextId, options);
```

`openRecoveryBackup` authenticates the ciphertext and checkpoint before returning a validated identity.
Wrong passwords, tampered files, different owners and changed scopes fail.
The optional `encryptionKeys` map stores explicitly supplied 32-byte X25519 secrets as hexadecimal values.
Applications must restore those separate keys when their recipient cards require them.
Backing up a signing identity alone does not recover a separate encryption key.
Losing both the keys and their backup leaves ciphertext unreadable.

## Native general-memory recovery

`mnemonic_core::rebuild::recover_memory` accepts original signed bytes, a pinned author and an optional X25519 secret.
It verifies the signature and author before selecting a supported kind.
Supported kinds are `memory.v1` and `sealed.v1`.
Sealed inner memory can use canonical CBOR or the existing SDK JSON representation.
The result preserves original envelope bytes and complete plaintext bytes.
It does not require an embedding to recover content.
Unknown kinds, author mismatches, missing keys and failed opens produce structured errors.

`fetch_recovered_memories` fetches known locators through a bounded, configured gateway.
Successful candidates survive other fetch or verification failures.
Use this API only in the client's process when opening private memories.
It does not write decrypted memory into an operator database.

`recover_memory_set` checks signed parent references and reports forks and missing parents.
Each `MemoryRecoveryHead` pins an identifier, author and exact envelope digest.
Supply equivalent pins for every ancestor through `trusted_ancestry`.
Memory parent references contain identifiers, so head pins alone cannot authenticate earlier artifact bytes.
The native sealed helper uses wraps inside the signed envelope; separate grant lookup uses existing client grant flows.
Conflicting signed envelopes with the same identifier cannot satisfy expected heads.
Cycles and ancestry deeper than the protocol limit cannot satisfy expected heads.
No supplied heads means `complete_to_heads` is false.

The legacy `mnemonic-mcp restore` command rebuilds its local index from supported public memory artifacts.
It reports sealed items as requiring client-local recovery instead of guessing a decryption key.
Its source report distinguishes exhaustion, partial results, failure and scan-budget exhaustion.
It retains candidates from successful pages when later pages fail.
Source exhaustion still does not establish global completeness.

## Validation limits

Native tests cover exact bytes, complete public and sealed plaintext, wrong keys, tampering, unsupported kinds and source outages.
They also cover partial scans, budgets, signed forks, missing parents, cycles and absent expected heads.
SDK tests use real WebAssembly cryptography and an encrypted identity backup in a fresh client.
They restore pinned A2A ancestry through a mocked gateway while the index is unavailable.
The test checks plaintext opening after identity restoration.
Live provider discovery, independent operator deployment and published-package drills remain separate release gates.
