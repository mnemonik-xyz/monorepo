# Tech spec: sealed memories (encrypted private memories and private sharing)

Status: draft for owner review, 2026-09-27. Nothing in this spec is implemented.
Every capability below is **planned** unless a line says "available now".

Related: issue #242 (encrypted memories over A2A), #233 (A2A bridge V1),
#211 (capability-scoped composition), #241 (agent-owned Arweave items).

## 1. Summary

Owner decision (2026-09-27): "private" means **encrypted**. This applies to
memories that the server stores (`local`) and to memories anchored on chain.
Earlier decisions (`work/presentable-mvp/plan.md:129-137`): shared memories are
private (ciphertext) by default. Public memories are plaintext.

This spec defines **sealed mode**:

1. The client encrypts each memory with a new random content key `K`.
2. The client wraps `K` to the author's X25519 key. This key comes from the Ed25519 identity.
3. The author signs the **encrypted** artifact. The anchor hash is the hash of
   the encrypted artifact, so nobody can confirm a guessed text.
4. Later, the author can give access to a chosen reader. A small signed grant
   record holds `K`, wrapped to the reader's key.
5. Semantic recall over sealed memories runs on the client, after decryption.

## 2. Current state (verified in code, 2026-09-27)

| Area | Fact | Source |
|---|---|---|
| Hosted write | The server builds the MEMORY_V1 artifact from the plaintext tool argument. It puts the TurboQuant embedding in `metadata.embedding_compressed`. | `mcp/src/tools.rs:877-919` |
| Anchoring | The sign-callback uploads the full COSE_Sign1 bytes to Arweave: plaintext `content` and the compressed embedding. | `mcp/src/api.rs:569-600` |
| Memo | The Solana memo is `{"h": content_hash, "a": ar_tx, "v": 2}`. `h` is the blake3 hash of the **plaintext** canonical CBOR. | `mcp/src/api.rs:645-649` |
| "Private" label | The callback saves every anchored row with `Visibility::Private`. The bytes on Arweave are plaintext. | `mcp/src/api.rs:766-784` |
| Visibility | `visibility` is an access filter on SQL rows only. The default is `private`. | `mcp/src/tools.rs:196-237`, `core/src/storage/mode.rs:124-128` |
| Local rows | Hosted `local` rows keep plaintext `content` and a plaintext f32 embedding on the server. | `core/src/storage/sqlite.rs:18`, `:31-34` |
| Encryption code | `core/src/encrypt.rs` has `seal_to_recipients` / `open` (X25519 + XSalsa20-Poly1305, full plaintext per recipient). No caller uses it. The Ed25519 to X25519 mapping is missing. | `core/src/encrypt.rs:1-29`, `:86-150`; `work/noncustodial-paradigm/IMPLEMENTATION_STATUS.md:155-160` |
| Schema | `MEMORY_V1` field order: `artifact_id, type, schema_version, content, metadata, parents, tags, created_at, producer`. | `core/src/codec/schema.rs:226-249` |
| Rebuild | `rebuild_row` reads the embedding from the signed artifact. The module notes that embeddings can leak content through inversion. | `core/src/rebuild.rs:17-25` |
| Recovery | Chain recovery reads `producer` from the COSE payload and from the `Producer` Arweave tag. | `core/src/arweave/recovery.rs:14-17` |
| Keychain rule | The CLI reads the private key only for anchored writes. `local` writes use only the public key. | `packages/cli/src/commands/sign.ts:3-11`, `docs/WHITEPAPER.md:142-144` |
| SDK | `signMemory` signs the pending canonical CBOR **verbatim**. It never encodes it again. | `packages/sdk/src/client.ts:6-22`, `:198-230` |

**Finding F1 (privacy gap in production now).** Anchored memories with the
label `private` are readable by anyone on Arweave. The label is only an SQL
filter. `docs/WHITEPAPER.md:101` already lists `sealed` as planned and says
that the current participate mode is "not encrypted". The UI and
`docs/tools.md` must not call these rows private. Task 6 fixes the label.
The bytes already on Arweave are permanent (`work/presentable-mvp/plan.md:137-139`).

**Finding F2 (hosted transport).** A hosted MCP client (for example a chat
connector) sends the memory text as a plain tool argument over TLS. The
server always sees this text while it handles the call. No encryption format
can change this. Only a client-side component (local MCP, SDK, CLI,
extension, webapp) can seal the text before it leaves the device.

## 3. Terms

| Term | Meaning |
|---|---|
| `K` | Content key: 32 random bytes, one per memory version. |
| Inner artifact | The plaintext MEMORY_V1 canonical CBOR (content, tags, parents, embedding). |
| Outer artifact | The public SEALED_V1 canonical CBOR. It holds the ciphertext of the inner artifact. |
| Wrap | `K`, encrypted to one X25519 public key with HPKE. |
| Grant | A signed record that gives one reader a wrap of `K`. |
| Bearer link | A URL that holds `K` in the fragment. Whoever holds the link can read the memory. |
| AEAD | Authenticated encryption with associated data. |
| HPKE | Hybrid Public Key Encryption, RFC 9180. |
| KEM | Key encapsulation mechanism. |
| KDF | Key derivation function. |
| ECDH | Elliptic-curve Diffie-Hellman key agreement. |
| E2E path | The client seals before it sends. The server never sees plaintext. |
| Hosted path | The server receives plaintext in the tool call, seals it, and deletes the plaintext. |

## 4. Modes after this change

| Mode (user word) | `mode` | `visibility` | Stored where | Who can read | Signed |
|---|---|---|---|---|---|
| local | `local` | `private` (default) | Server SQLite: SEALED_V1 ciphertext only | Author | No (as today) |
| sealed | `participate` | `private` (default) | Arweave ciphertext + Solana hash + server cache | Author + granted readers | Yes, over ciphertext |
| public | `participate` | `public` | Arweave plaintext + Solana hash | Anyone | Yes, over plaintext (as today) |

On-device stores (stdio local MCP, extension) are the client. Their `local`
rows can stay plaintext on the device (owner decision D-6).

`visibility: "private"` keeps its name on the wire. Its meaning changes from
"SQL filter" to "encrypted". A `local` write may now send `visibility`
(today it is rejected, `mcp/src/tools.rs:208-227`); only `private` is legal.

## 5. Cryptography

### 5.1 Content encryption: XChaCha20-Poly1305 (recommended)

- `K` = 32 bytes from the OS random source. One `K` per memory version.
- Cipher: XChaCha20-Poly1305. Nonce: 24 random bytes per encryption.
- Plaintext: the inner MEMORY_V1 canonical CBOR, padded (§5.6).
- Implementation: RustCrypto `chacha20poly1305` crate, type `XChaCha20Poly1305`.
  The crate received one security audit by NCC Group with no significant
  findings (<https://docs.rs/chacha20poly1305/latest/chacha20poly1305/>).
  The SDK, webapp and extension use the same Rust code through WASM. There is
  no second implementation in JavaScript.

Why XChaCha20-Poly1305 and not AES-GCM:

| Point | XChaCha20-Poly1305 | AES-256-GCM |
|---|---|---|
| Nonce | 192 bits. Random nonces are safe: "a single collision (with probability 50%) after roughly 2^96 messages" (<https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha>). | 96 bits. Random nonces need a usage limit per key. |
| Streams and re-seal | Random or counter nonces both work (A2A streams, §9.4). | Needs careful counter management. |
| WASM | ChaCha20 uses only add, rotate and XOR on 32-bit words. It does not need hardware AES instructions. | Fast and constant-time only with hardware AES support. |
| Codebase fit | Same family as the XSalsa20 `crypto_box` already in `core/src/encrypt.rs`. | New family. |
| Standard status | The XChaCha draft is an expired Internet-Draft (version 03, January 2020). ChaCha20-Poly1305 itself is standard. | NIST standard. |

The expired draft status is the main cost. The construction is simple
(HChaCha20 subkey + ChaCha20-Poly1305) and the audited crate implements it.

**Associated data (AD).** The content hash cannot be in the AD: the hash
covers the ciphertext. The AD is the canonical CBOR of the outer header:
`{v, alg, artifact_id, producer, created_at, kc}` (§6.1). Result: a
ciphertext moved into another artifact, or into another author's name, fails
to decrypt.

**Key commitment.** Poly1305 AEADs are not key-committing: one ciphertext can
decrypt correctly under two different keys when the attacker picks both keys.
A dishonest author could give two readers two keys that open two different
texts. Fix: the signed outer artifact holds
`kc = blake3::derive_key("mnemonic sealed v1 key commitment", K)`.
A reader rejects a `K` whose `kc` does not match.

### 5.2 Key wrapping: HPKE base mode (recommended)

Owner requirement: key agreement uses Diffie-Hellman or a similar protocol.

Recommended: **HPKE base mode, RFC 9180** (<https://www.rfc-editor.org/rfc/rfc9180>),
single-shot API (§6 of the RFC), cipher suite:

- KEM `DHKEM(X25519, HKDF-SHA256)`, ID `0x0020`
- KDF `HKDF-SHA256`, ID `0x0001` (HKDF: RFC 5869, <https://www.rfc-editor.org/rfc/rfc5869>)
- AEAD `ChaCha20Poly1305`, ID `0x0003`

DHKEM is X25519 ECDH (RFC 7748, <https://www.rfc-editor.org/rfc/rfc7748>)
with a new ephemeral sender key for each wrap, followed by HKDF. This is the
ECIES pattern in a standard form. Each wrap has its own ephemeral key, so a
leak of one wrap secret does not expose other wraps.

Wrap inputs:

- `pkR` = the reader's X25519 public key (§5.3).
- `info` = `"mnemonic/sealed/v1/wrap" || ct_hash || pkR`, where
  `ct_hash = blake3(nonce || ciphertext)`. HPKE uses `info` "to fold in
  identity information" (RFC 9180 §5.1.1). This binds the wrap to one
  ciphertext and one reader.
- `aad` = UTF-8 bytes of the author DID (`did:sol:<base58>`).
- `pt` = `K` (32 bytes). Output: `enc` (32 bytes) + wrapped `K` (48 bytes).

Why HPKE and not a custom "X25519 + HKDF + AEAD" ECIES:

| Option | For | Against |
|---|---|---|
| **HPKE base mode** (recommended) | Standard. Published test vectors for this exact suite: RFC 9180 Appendix A.2. The RFC requires the all-zero shared-secret check for X25519 (§7.1.4). The Rust `hpke` crate implements RFC 9180 with X25519 and ChaCha20Poly1305 (<https://docs.rs/hpke/latest/hpke/>). | One more dependency. We found no public audit statement for the `hpke` crate. The security audit task (16) must review it. |
| Custom ECIES (X25519 + HKDF-SHA256 + XChaCha20-Poly1305) | Few lines. Reuses crates we already have. | We design the KDF labels and test vectors ourselves. Easy to get domain separation wrong. |
| Keep `crypto_box` sealing of the full plaintext (current `encrypt.rs`) | Code exists. | Cannot grant later without the full plaintext. The ciphertext grows by one full copy per reader. |

Fallback: if the audit rejects the `hpke` crate, implement DHKEM + HKDF
directly from RFC 9180 §4.1 and check it against the Appendix A.2 vectors.
The wire format stays the same.

### 5.3 X25519 keys: birational map (recommended) vs separate keys

Option K1 — **derive from the Ed25519 identity** (recommended for v1).
RFC 7748 §4.1 gives the birational map between edwards25519 and curve25519
(`u = (1+y)/(1-y)`). libsodium exposes it as
`crypto_sign_ed25519_pk_to_curve25519` and `crypto_sign_ed25519_sk_to_curve25519`
(<https://doc.libsodium.org/advanced/ed25519-curve25519>). In Rust,
`ed25519_dalek::VerifyingKey::to_montgomery` gives the public key
(<https://docs.rs/ed25519-dalek/latest/ed25519_dalek/struct.VerifyingKey.html>),
and `SigningKey::to_scalar_bytes` gives the secret scalar.

- For: anyone who knows a DID can seal to it. No key directory, no discovery
  step, one secret to back up. The keychain holds nothing new.
- For: Thormarker proves the joint security of Ed25519 and an X25519 KEM with
  an HKDF-Extract-like KDF on the same key pair, in the random oracle model
  (<https://eprint.iacr.org/2021/509>). DHKEM uses HKDF-Extract.
- Against: libsodium says "using distinct keys for signing and for encryption
  is still highly recommended" if you can afford it. A compromise of the
  identity key opens every sealed memory. The encryption key cannot rotate
  without a new identity.

Option K2 — **separate X25519 key** from the seed (`HKDF(seed, "enc")`, as in
`work/noncustodial-paradigm/design.md:569-594`), published in a signed
`enc_key` record (AgentCard `x-mnemonic`, server key directory, later
`did:mnemonic`).

- For: key separation; the encryption key can rotate.
- Against: senders must discover and trust the record. A stale or forged
  record breaks sealing. More work for every client.

Recommendation: K1 now. Reserve an optional signed `enc_key` record (K2) as
planned. When a valid `enc_key` record exists, senders use it; otherwise they
use the mapped key. Owner decision D-2.

Checks on every X25519 operation: reject the all-zero shared secret
(RFC 9180 §7.1.4 makes this a MUST for DHKEM). Reject a public key that maps
from an Ed25519 point of small order.

### 5.4 Key rotation

- **New content version.** An edit is a new memory with a new `K`. It links
  to the old one with `parents` (inside the ciphertext).
- **Identity rotation.** Old wraps stay tied to the old key. The author must
  open each memory with the old key and grant it to the new key (self-grant).
  Memories not re-granted before the old key is lost stay closed forever.
- **K2 enc-key rotation** (planned): same self-grant procedure, identity stays.

### 5.5 Revocation limits

A reader who received `K` and saved it keeps access forever. Nobody can
revoke this. The author can only:

- stop serving a grant from the server database (stops new discovery);
- make a new version with a new `K` and not grant it to that reader.

Grants anchored on Arweave are permanent. Docs and UI must say this in plain
words before a user grants access.

### 5.6 Length hiding

The ciphertext length shows the plaintext length. Pad the inner CBOR to the
next multiple of 256 bytes before encryption (a padding length field goes
first). This hides small differences, not large ones.

## 6. Artifact format, signing and hashing

### 6.1 SEALED_V1 (outer artifact)

New artifact type `sealed`, schema version 1, registered as
`("sealed", 1) => SEALED_V1` next to `MEMORY_V1` (`core/src/codec/schema.rs:414`).
`MEMORY_V1` does not change. Its golden fixtures stay valid.

| Field | Type | Public | Meaning |
|---|---|---|---|
| `artifact_id` | text (UUID) | yes | Same role as in MEMORY_V1. |
| `type` | `"sealed"` | yes | |
| `schema_version` | `1` | yes | |
| `alg` | `"xchacha20poly1305"` | yes | Content AEAD. |
| `nonce` | bstr(24) | yes | Content nonce. |
| `ct` | bstr | yes | AEAD ciphertext of the padded inner MEMORY_V1 CBOR. |
| `kc` | bstr(32) | yes | Key commitment (§5.1). |
| `wraps` | array | yes | Wraps made at write time. v1: exactly one, to the author. Each: `{kid, enc, wk}`. `kid` = the X25519 public key. |
| `created_at` | text | yes | Needed for anchoring and recovery. |
| `producer` | text (DID) | yes | Author. Needed for verification and recovery. |

Canonical field order: `artifact_id, type, schema_version, alg, nonce, ct, kc,
wraps, created_at, producer`.

Inner MEMORY_V1 holds `content`, `tags`, `parents` and `metadata` (with
`embedding_compressed`). These fields are **not** in the outer artifact:
tags and parent links would leak topics and structure.

The inner `producer` and `artifact_id` must equal the outer ones. A reader
checks this after decryption.

### 6.2 What is signed

Recommended: **encrypt, then sign** (one signature).

- COSE_Sign1 (Ed25519, as today, `core/src/codec/sign.rs:53-81`) over the
  outer canonical CBOR. The payload is the outer artifact.
- The inner artifact has no separate signature.
- Plaintext authenticity for a reader comes from three checks together:
  the author signed the ciphertext, AEAD decryption with `K` succeeds, and
  `kc` matches `K`.

Options considered:

| Option | For | Against |
|---|---|---|
| **Sign outer only** (recommended) | One signature. The hosted deferred flow stays one round-trip. Third parties can verify. | A reader who forwards the plaintext cannot prove authorship to others without also giving `K` and the outer artifact. |
| Sign inner, then encrypt, then sign outer | The decrypted plaintext carries its own signature. | Two signatures. The hosted flow needs two signing rounds. |
| Sign plaintext only, encrypt the COSE | Hides the author. | Third parties cannot verify authorship or existence. The anchor loses its value. |

A forwarded proof works like this: the reader gives the outer COSE and `K`.
Anyone can then verify the signature, check `kc`, decrypt and read.

### 6.3 What is hashed and anchored

- `content_hash = blake3(outer canonical CBOR)`. It covers the random nonce,
  the ciphertext and the wraps.
- The Solana memo keeps its shape: `{"h": content_hash, "a": ar_tx, "v": 3}`.
  `v: 3` tells readers that `h` can be a sealed artifact.
- Nobody can confirm a guessed short secret from `h`: `h` depends on `K` and
  on the nonce, which are random.
- Arweave tags: `Producer`, `Created-At` (as today) and `Mnemonic-Type: sealed`.
  No other tag.

A keyed hash of the plaintext was considered as the anchor. It is rejected:
it needs a second secret, and it adds nothing that the ciphertext hash does
not give.

### 6.4 Verification for third parties (no key)

Anyone can check, with no key:

1. The COSE_Sign1 signature is valid for `kid` = the `producer` key.
2. `blake3(payload) == h` in the Solana memo.
3. The memo block time gives the date.

The verifier learns: author, date, ciphertext size (padded), and the number
of write-time wraps. The verifier does not learn: text, tags, embedding,
parents. `mnemonic_verify` returns `sealed: true, readable: false` for this case.

### 6.5 Grant record (GRANT_V1)

New artifact type `grant`, schema version 1.

| Field | Meaning |
|---|---|
| `type`, `schema_version` | `"grant"`, `1` |
| `memory_hash` | `content_hash` of the sealed memory |
| `reader` | Reader X25519 public key (or omitted for an anonymous grant, §8.2) |
| `enc`, `wk` | HPKE output for `K` to the reader (§5.2) |
| `perms` | Optional reference to a capability token (§8.3) |
| `created_at`, `producer` | Date and author DID |

The author signs the grant with COSE_Sign1. Issue #242 names the fields
`{memory_hash, reader_pubkey, ephemeral_pubkey, wrapped_K}`. They map to
`memory_hash`, `reader`, `enc`, `wk`.

## 7. Semantic recall over sealed memories

### 7.1 The constraint

Semantic recall needs an embedding. An embedding leaks the text: an
inversion method recovered "92% of 32-token text inputs exactly" from dense
text embeddings (Morris et al., <https://arxiv.org/abs/2310.06816>).
`core/src/rebuild.rs:23-25` notes the same risk. So a plaintext embedding
stored on the server is not private.

A hosted server can either read a memory or not. It cannot run plaintext
semantic search over memories that it cannot read. Fully homomorphic
encryption and secure enclaves could change this; both are out of scope (§12).

### 7.2 Options

| Option | How | Privacy | Ease for hosted connectors |
|---|---|---|---|
| (a) **Client-side recall** | The client embeds, encrypts the embedding **inside** the inner artifact, and keeps a local decrypted index. The server stores ciphertext only. | Strong. Server never reads memories at rest. | Local MCP, CLI, SDK, extension: good. Pure hosted connector: no sealed recall. |
| (b) Server embeds, stores ciphertext + plaintext embedding | As today, content encrypted. | Weak. The embedding leaks the text (inversion). Does not meet the owner rule. | Good. |
| (c) No recall for sealed memories | Sealed rows are write-only on the server. | Strong. | Poor. |
| (d) Hosted recall session (opt-in) | The owner unlocks a per-owner recall key `RK` for a time window. The server holds `RK` in RAM only and decrypts in memory. | At rest: strong. During a session: the server can read. | Good. |

**Recommendation: (a) as the base, (d) as an explicit opt-in** (owner
decision D-3). Reject (b): it breaks "private means encrypted". (c) is what a
pure hosted connector gets when (d) is off.

### 7.3 Client-side recall (a), details

- **Where the embedding lives.** In the inner artifact `metadata`
  (`embedding_compressed`, optional `embedding_f32`). It is encrypted with
  the content. Item 6 of the brief: the compressed embedding is never
  published in plaintext for sealed memories.
- **Who embeds.** Local MCP: `fastembed`, as today. Hosted path: the server
  embeds the plaintext it already has (F2), puts the embedding inside the
  inner artifact, then seals. SDK, CLI and extension on the E2E path: use a
  local embedder when one is set. Otherwise they ask the server
  `POST /api/embed` for the vector (the server sees that one text
  transiently, not the stored memories). Owner decision D-4.
- **Index rebuild on a new device.** `GET /api/sealed?owner=<me>` returns the
  owner's SEALED_V1 blobs. The client decrypts them and builds its local
  index. Chain recovery (`core/src/arweave/recovery.rs`) also finds them;
  `rebuild_row` gets a `rebuild_sealed_row(bytes, x25519_secret)` variant.
- **Server recall.** The server vector search skips sealed rows (they have no
  row in `attestation_embeddings`). `mnemonic_recall` adds
  `sealed_hidden: <count>` and a hint, so the agent knows that more memories
  exist on the client side.

### 7.4 Hosted recall session (d), details (opt-in, planned)

- A per-owner random key `RK`. The server stores `RK` wrapped to the owner
  X25519 key (only the owner can unwrap it).
- For each hosted sealed row the server also stores: `K` wrapped under `RK`
  and the f32 embedding encrypted under `RK`. These live in SQLite only,
  never on Arweave.
- Unlock: the webapp or SDK unwraps `RK` with the owner key and sends it to
  `POST /api/recall-session` with a time limit (default 60 minutes, owner
  decision). The server keeps `RK` in memory only, never on disk or in logs.
- During the session, `mnemonic_recall` decrypts embeddings in memory, ranks,
  decrypts the top results and returns plaintext.
- Honest label in UI and docs: "While a recall session is on, the Mnemonic
  server can read your private memories in memory."

### 7.5 Keychain and unlock

Today the keychain is read only to sign (`docs/WHITEPAPER.md:142-144`).
Sealing to yourself needs only your **public** key. Opening needs the secret.

New rule: the client reads the secret key to **sign** (anchor, publish, prove,
grant) and to **open** sealed memories. It never reads it for a local sealed
write or for a public recall.

Unlock cache for local clients (local MCP, CLI, extension):

- On the first open in a process, read the identity secret once, derive the
  X25519 secret, and hold it in memory (zeroized on drop, `zeroize` crate).
- Time limit `MNEMONIC_UNLOCK_TTL` (default: process lifetime for local MCP;
  15 minutes for the extension). After it expires, the next open asks again.
- Decrypted `K` values and plaintexts go into the local index. On-device
  stores are protected by the OS (owner decision D-6).
- Result: at most one keychain prompt per session for recall.

## 8. Write, local storage and sharing flows

### 8.1 Hosted write, `participate` + `private` (sealed)

The client flow stays the deferred-signing flow (`packages/sdk/src/client.ts:6-22`).

1. The agent calls `mnemonic_sign_memory {content, mode: "participate"}`.
2. The server embeds and compresses (as today), builds the inner MEMORY_V1
   CBOR with `producer = did:sol:<jwt.sub>`, and pads it.
3. The server makes `K` and a nonce, encrypts, computes `kc`, and wraps `K`
   to the owner X25519 key. It needs only the owner **public** key (§5.3 K1).
4. The server builds the outer SEALED_V1 CBOR and `content_hash`. It parks
   **only** the outer CBOR in `PendingBundles`. It zeroizes the plaintext,
   the inner CBOR and `K` (`zeroize` crate). The embedding is dropped.
5. The client fetches `GET /api/pending/{id}` (outer CBOR, as today).
   **New check:** the client unwraps `K`, checks `kc`, decrypts, and compares
   the text to what it sent (SDK) or shows it to the human (webapp approve
   page). This stops a server that seals other text.
6. The client signs the outer CBOR verbatim and posts `/api/sign-callback`.
7. The server verifies, uploads the COSE bytes to Arweave with tag
   `Mnemonic-Type: sealed`, writes the memo (`v: 3`), runs the delivery
   check on the outer hash, and saves the row: `content = ''`,
   `sealed_blob = COSE bytes`, no row in `attestation_embeddings`.

Today `PendingBundles` keeps plaintext for up to 300 s and the callback
saves `entry.content` (`mcp/src/api.rs:771-784`). This must change: a
sealed pending entry holds no plaintext.

### 8.2 E2E write (local MCP, SDK, CLI, extension)

1. The client embeds (local embedder or `POST /api/embed`, D-4), builds the
   inner CBOR, seals, and signs the outer CBOR.
2. `mode: participate`: the client posts the signed COSE to a new route
   `POST /api/anchor-sealed`. The server verifies the signature and the
   payer or quota, uploads and anchors. The server never sees plaintext.
3. `mode: local`: the client posts the unsigned outer CBOR to
   `POST /api/store-sealed`. The row is owned by `jwt.sub`.

Both routes reject a payload whose `producer` is not `did:sol:<jwt.sub>`.

### 8.3 Local (server-stored) private memories

Brief item 4. A hosted `local` write with `visibility: private` (the default)
stores a SEALED_V1 blob in SQLite, not plaintext:

- Same format as §6.1. No signature (local rows are unsigned today).
- Sealing needs only the owner public key. The write needs no client round-trip
  and no keychain prompt. The hosted UX for writes does not change.
- The server keeps no plaintext and no plaintext embedding.
- Recall: client-side (§7.3), or a hosted recall session when the owner turned it on (§7.4).
- A later `participate` of the same memory (planned "anchor later") signs the
  existing outer CBOR. No re-encryption.

### 8.4 Grants (sharing with chosen agents)

Brief item 5. Only a holder of the author secret can unwrap `K`, so every
grant is made on the client.

1. The author picks a reader: DID, X25519 key, AgentCard, or "link".
2. The client unwraps `K` (unlock cache, §7.5), wraps it to the reader
   (§5.2), builds GRANT_V1, and signs it.
3. Where the grant goes (owner decision D-5):

| Option | For | Against |
|---|---|---|
| **G1 Server DB** `POST /api/grants` (recommended default) | Fast discovery: `GET /api/grants?reader=<kid>`. Can stop serving it later. | Server learns who shares with whom. |
| G2 Arweave item (+ memo) | Permanent, no server needed. | Public social graph (unless anonymous). Costs one quota unit. Cannot be withdrawn. |
| G3 Out of band (A2A message, file, link) | Server learns nothing. | Reader must receive it directly. |

Recommendation: G1 + G3 by default, G2 as opt-in. An **anonymous grant**
omits `reader`. The reader tries its key on each candidate grant. Use it for G2.

Hosted connector path: `mnemonic_share {memory_hash, reader}` returns
`awaiting_signature` with an `approve_url`. The webapp does steps 2-3 in the
browser. The server never sees `K`.

### 8.5 Bearer links

`https://mnemonik.xyz/m/<content_hash>#k=<base64url(K)>`. The fragment is
"dereferenced solely by the user agent" (RFC 3986 §3.5,
<https://www.rfc-editor.org/rfc/rfc3986#section-3.5>), so the browser does not
send `K` to the server.

- The `/m/` page loads the outer artifact, checks the signature and `kc`, and
  decrypts in the browser (WASM). It loads no third-party scripts.
- The reader's agent can import the link: it wraps `K` to its own key
  (a self-grant stored locally or in G1).
- Risks: the link leaks through chat logs, browser history and browser
  extensions. A link is full read access forever. The UI says so.

### 8.6 How a recipient discovers and opens

1. Discovery: `GET /api/grants?reader=<my kid>` (G1), an A2A message (G3),
   an Arweave query for anonymous grants (G2), or a bearer link.
2. The recipient checks the grant signature (the `producer` must be the
   memory author), fetches the sealed memory (server cache or Arweave),
   verifies it (§6.4), unwraps `K`, checks `kc`, and decrypts.
3. Foreign plaintext enters recall only through explicit import, with
   provenance and spotlighting (layers 1, 2, 4 in
   `work/presentable-mvp/plan.md:186-199`). A trusted-author list (layer 3)
   is planned.

### 8.7 Capability tokens

`docs/spec/memory-composition.md` §2 defines a signed `capability.token`
(`subject`, `scope`, `permissions`, `expiry`, `revocation_reference`,
`chain_of_authority`). YELLOWPAPER §7.2 (`docs/YELLOWPAPER.md:529-541`)
summarizes it. Relation to grants:

- A **grant delivers the key**. A **token states the permission**.
- A grant may carry `perms` = the hash of a capability token.
- Cryptography enforces only "can decrypt". `expiry`, `quote` and
  `share-onward` are **advisory** once the reader has `K`. Honest runtimes
  check them. A dishonest reader can ignore them. Docs must say this.
- Token revocation (§2.3 of that spec) stops a server or runtime from serving
  new data. It cannot take back a `K` already delivered.
- Issue #211 owns the token implementation. This spec only reserves the
  `perms` field.

## 9. A2A payloads (issue #242)

The sharing handshake in `docs/spec/memory-composition.md` §3 and
YELLOWPAPER §7.3 (`docs/YELLOWPAPER.md:543-553`) uses ECDH for a transport
session. Sealed memories need **no live session**: the grant itself is the
key delivery. A2A only moves bytes.

### 9.1 Payload

An A2A message or artifact that carries a memory holds a data part:

- media type `application/vnd.mnemonic.sealed+cbor` (base64 in JSON),
- content: CBOR map `{sealed: <outer COSE_Sign1 bytes>, grants: [<GRANT_V1 COSE>]}`,
  with one grant per A2A recipient.

The A2A attestation (#233 task 1, `A2A_MESSAGE_V1`) covers the payload hash,
never the plaintext.

### 9.2 Recipient key discovery

1. AgentCard `x-mnemonic` extension (#233 task 4,
   `work/a2a-bridge/tech-spec.md:63-65`): `ed25519_pubkey_base58`. Map it to
   X25519 (§5.3 K1).
2. Optional `x-mnemonic.enc_key`: a signed X25519 key (K2, planned). Use it
   when valid.
3. `did:mnemonic` resolution (#74, planned).

The AgentCard JWS must cover the extension (`work/a2a-bridge/tech-spec.md:108`).
Without a verified card, the sender must not seal to that key.

### 9.3 Tools

`mnemonic_attest_a2a` and `mnemonic_recall_a2a` (#233 task 5) get `sealed`
input and output. Decryption runs only in the client. The server never sees
plaintext or unwrapped keys (#233 constraints).

### 9.4 Streams (#61)

For an attested stream, seal each chunk with the same `K`:

- nonce = 19 random bytes (per stream) || 4-byte chunk index (big-endian) || 1 byte last-chunk flag;
- AD = outer header AD || stream id || chunk index || last-chunk flag.

A reordered, dropped or truncated chunk fails to decrypt. The final chunk
must carry the flag. Each chunk hash goes into the #61 hash chain.

## 10. Migration and compatibility

Brief item 7.

- **Existing rows.** Plaintext rows stay readable. They get
  `privacy = 'plaintext'` in a new column. Hosted `local` plaintext rows:
  owner decision D-7 (options: keep, seal in place with the owner key, or
  delete the plaintext after the owner exports).
- **Mislabel fix (F1).** Existing anchored rows labelled `private` become
  `privacy = 'plaintext'`, `visibility = 'public'` in the API. The UI shows
  "published before encryption existed; readable on Arweave".
- **Schema.** New types `sealed` v1 and `grant` v1. No MEMORY_V2: the inner
  artifact stays MEMORY_V1. Memo `v: 3`.
- **SQLite.** New columns `privacy TEXT NOT NULL DEFAULT 'plaintext'`,
  `sealed_blob BLOB`. New tables `grants`, and for §7.4 `owner_recall_keys`
  and `sealed_index`. All SQL parameterized (`core/src/storage/sqlite.rs`).
- **Legacy clients.** A client that sends no `visibility` on `participate`
  now gets a sealed bundle to sign. The SDK signs bytes verbatim, so signing
  still works. Old SDKs skip the step-5 check; the new SDK adds it.
- **Golden fixtures.** Fixed `K`, nonce and HPKE ephemeral key (HPKE
  `DeriveKeyPair` from a fixed seed) give fixed bytes. Emit them from
  `core/tests/golden_fixtures.rs` and check them in the SDK (WASM) and the
  extension. Also test the RFC 9180 Appendix A.2 vectors.
- **Recovery.** `recovery.rs` finds sealed items by memo and `Producer` tag.
  Traction stats count them. `rebuild_row` skips them without a key.
- **Clients.** SDK: `signMemory` check, `sealMemory`, `openMemory`,
  `share`, `importLink`, `recallSealed`. CLI: `mnemonic sign` default sealed,
  `mnemonic share`, `mnemonic open`, `mnemonic recall` with local index.
  Extension: seal before cloud sync. Webapp: approve page decrypt, `/m/`
  sealed view, grant UI, recall-session toggle.

## 11. Threat model update

New boundary "Sealed memories" for
`.claude/skills/project-knowledge/references/threat-model.md` (task 15).

| # | Attack | Defeating check | Class |
|---|---|---|---|
| S1 | Arweave reader or gateway reads a sealed memory | Content is XChaCha20-Poly1305 ciphertext. Tags, parents and embedding are inside it. | Confidentiality |
| S2 | Guess a short secret and compare with the anchor hash | `h` covers random nonce, `K` output and wraps (§6.3). | Confidentiality |
| S3 | Server operator reads stored private memories | E2E path: server never has plaintext. Hosted path: plaintext only during the call (F2); stored as ciphertext. | Confidentiality at rest; transient exposure on the hosted path |
| S4 | Server seals different text on the hosted path | Client decrypts the pending bundle and compares before it signs (§8.1 step 5). | Integrity |
| S5 | Author gives two readers keys that open two different texts | `kc` in the signed outer artifact (§5.1). | Integrity |
| S6 | Move a ciphertext or wrap into another artifact or author | Content AD holds `artifact_id`, `producer`; HPKE `info` holds `ct_hash` and reader key. | Integrity |
| S7 | Forged grant | Grant COSE signature must match the memory `producer`. | Integrity |
| S8 | Sender seals to a forged recipient key (A2A) | Use only keys from a verified AgentCard JWS or the mapped DID key. | Integrity |
| S9 | Small-order or all-zero X25519 key | Reject all-zero shared secret (RFC 9180 §7.1.4); reject small-order input keys. | Integrity |
| S10 | Embedding inversion | No plaintext embedding stored for sealed rows (§7). | Confidentiality |
| S11 | Social graph from grants | G1 default (server only), anonymous grants for G2. The server still sees G1 metadata. | Metadata, partly mitigated |
| S12 | Length and timing analysis | 256-byte padding. Dates and sizes stay public. | Metadata, partly mitigated |
| S13 | Identity key theft | Opens every sealed memory (K1). K2 limits this for future memories only. | Out of scope (keychain boundary) |
| S14 | Recall-session abuse | `RK` in RAM only, time limit, opt-in, audit log of sessions. During a session the server can read. | Accepted risk, opt-in |
| S15 | Bearer link leak | Documented. Link = permanent read access. | Accepted risk |
| S16 | Reader keeps `K` after "revocation" | Not preventable (§5.5). | Non-goal |

## 12. Non-goals

- Revocation of a key that a reader already has.
- Hiding the author, date or approximate size of an anchored sealed memory.
- Server-side semantic search over sealed memories without a recall session.
- Fully homomorphic encryption, private information retrieval or secure
  enclaves (TEE). Possible later research only.
- Delegation without the author online (proxy re-encryption) and time-release
  encryption. Listed as P2+ in `work/presentable-mvp/plan.md:162-166`.
- Post-quantum key exchange. The HPKE suite ID leaves room for a later
  hybrid KEM.
- Protecting a device that is already compromised.
- Deleting data already on Arweave.

## 13. Testing

- Unit (core): seal/open round-trip; wrong key fails; tampered `ct`, `nonce`,
  `wk`, `enc`, AD or `kc` fails; all-zero shared secret rejected;
  Ed25519-to-X25519 map matches libsodium output for fixed keys.
- Vectors: RFC 9180 Appendix A.2 (HPKE suite); project golden vectors for
  SEALED_V1 and GRANT_V1 (Rust emits, SDK WASM and extension check).
- Property: random plaintexts and tag sets round-trip; padding hides length
  within one 256-byte bucket.
- Integration (mcp, `test-support`): hosted sealed participate end-to-end
  with a mock Arweave; pending entry holds no plaintext; saved row has
  `content = ''` and no embedding row; memo `v: 3`; `mnemonic_verify`
  returns `sealed: true`; recall returns `sealed_hidden`.
- SDK: step-5 mismatch rejects the signature; `recallSealed` finds a memory
  after an index rebuild from `GET /api/sealed`.
- Webapp e2e (Playwright): approve page shows decrypted text; `/m/#k=` opens
  in the browser and the network log shows no request with `K`.
- Audit: grep that no log line, error or metric contains plaintext, `K`,
  `RK` or a secret key.

## 14. Tasks

| # | Title | Wave | Size | Depends on | Main files |
|---|---|---|---|---|---|
| 1 | Core sealed crypto primitives | 1 | L | — | `core/src/sealed/`, `core/src/encrypt.rs`, `core/Cargo.toml`, `core/src/lib.rs` |
| 2 | SEALED_V1 and GRANT_V1 schemas | 1 | M | — | `core/src/codec/schema.rs`, `core/src/codec/sign.rs` |
| 3 | Seal/open/grant/link API and golden vectors | 2 | M | 1, 2 | `core/src/sealed/`, `core/tests/golden_fixtures.rs` |
| 4 | SQLite migration for sealed rows and grants | 2 | M | 2 | `core/src/storage/` |
| 5 | WASM bindings for sealed operations | 3 | S | 3 | `core/src/wasm/` |
| 6 | Hosted sealed write path and label fix | 3 | L | 3, 4 | `mcp/src/tools.rs`, `mcp/src/api.rs`, `mcp/src/pending.rs` |
| 7 | SDK sealed support | 5 | M | 5, 6, 10 | `packages/sdk/` |
| 8 | Webapp: approve decrypt, sealed view, links, grants | 5 | L | 5, 6, 10 | `webapp/` |
| 9 | Extension: seal before sync | 5 | M | 5, 10 | `packages/extension/` |
| 10 | MCP share, anchor-sealed, store-sealed, grants, sealed-aware recall/verify | 4 | L | 6 | `mcp/src/tools.rs`, `mcp/src/api.rs`, `mcp/src/mcp.rs` |
| 11 | CLI sealed commands and local index | 6 | M | 7, 10 | `packages/cli/` |
| 12 | Local MCP client-side sealed recall and unlock cache | 5 | M | 10 | `mcp/src/tools.rs`, `core/src/identity/` |
| 13 | Hosted recall session (opt-in) | 6 | L | 12, D-3 | `mcp/src/`, `core/src/storage/`, `webapp/` |
| 14 | A2A sealed payloads (#242) | 6 | M | 3, 10, a2a-bridge 1/4/5/7 | `core/src/codec/a2a/`, a2a crate |
| 15 | Docs consolidation and threat model | 7 | M | 6-12 | `docs/`, `.claude/skills/project-knowledge/references/`, `CLAUDE.md`, `AGENTS.md` |
| 16 | Audit: cryptography and secret handling | 8 | M | 1-15 | read-only, writes `decisions.md` |
| 17 | Audit: cross-language parity and migration | 8 | S | 1-15 | read-only, writes `decisions.md` |

Each code task also updates the docs that describe its behaviour (CLAUDE.md
"Docs change with code"). Task 15 checks consistency across all docs.

Shared-file conflicts: tasks 6, 10, 12, 13 all touch `mcp/src/tools.rs`, so
they are in different waves. Task 1 alone edits `core/src/lib.rs` in wave 1.

## 15. Open owner decisions

See `decisions.md` (D-1 to D-11). Wave 1 waits for D-2 and D-9. Task 6
waits for D-1, D-7 and D-8. Task 13 waits for D-3.
