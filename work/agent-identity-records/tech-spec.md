---
created: 2026-10-05
updated: 2026-10-05
status: draft
type: feature
size: L
---

# Tech Spec: Agent identity records

Status: **planned**. Mnemonik becomes the source of agent identity: an agent's
owner publishes a self-signed record chain through Mnemonik, and anyone resolves
and verifies it by the agent `id`. The Mnemonik operator cannot forge or replace
a record. It can only delay or hide one (section "Resolve").

## Goal

- One stable identifier per agent: the digest of its first record.
- The current signing key, encryption key, endpoints and AgentCard of that agent,
  resolvable by anyone from the identifier.
- Key rotation that a stolen current key cannot hijack.
- Old memories and attestations stay verifiable after a rotation, by anchor time.

Today an integration must exchange AgentCards and pin a "trusted card signer"
out of band (`docs/sealed-a2a.md`), and key rotation has no design.

## Design principles

1. **Self-certifying.** The identifier commits to the first key and to the first
   pre-committed next key. Only those keys, and keys that they pre-commit, can
   extend the chain.
2. **Mnemonik is an index, not an authority.** Clients verify every chain.
3. **Key roles.** Rotation needs the next key, which the owner keeps off the agent
   host. Whoever holds the current key, including the agent host, can publish
   updates and provisional revocations (rule 7). The MCP surface is read-only. A
   separate control key for records is planned.
4. **Trust is separate.** A valid record proves control of a key. Whether to deal
   with that agent is a policy decision (allowlists, endorsements, reputation).

## Record: `AGENT_RECORD_V1`

JSON in JCS form:

```text
{
  protocol:      "mnemonic.agent.record.v1"
  id:            hex blake3 of the JCS of record seq 0 with `id` left out
  seq:           integer, 0 for the first record
  prev:          hex blake3 of the signed payload bytes (JCS) of record seq-1; null at seq 0
  key:           base58 Ed25519 public key that signs this record
  next_key_hash: hex blake3 of the 32 raw bytes of the next Ed25519 public key
  enc_key:       base64 X25519 public key, optional
  agent_card:    A2A AgentCard, optional; carries a detached JWS by `key`
  services:      [{ type: "a2a" | "mcp" | "https", url }], optional
  controller:    id of the owner's own identity record, optional
  accounts:      KEY_BINDING_V1 proofs, planned (see below)
  created_at:    RFC 3339
  valid_until:   RFC 3339, at most 90 days after created_at
  revoked:       bool
}
```

Limits: 16 KiB per record, 16 services.

**Signing.** A new function `sign_record` signs the JCS bytes with COSE_Sign1,
EdDSA, and the protected content type `application/mnemonic-agent-record+json`.
`verify_record` requires that content type, rejects an unprotected header label
other than `kid`, and rejects a payload that differs from its JCS re-encoding.
`sign_cose` and the pending-bundle signer never set this content type. So a
signature that a client makes in another Mnemonik flow, for example the legacy
HTTP `mnemonic_sign_memory` flow, never verifies as a record.

**AgentCard.** If `agent_card` is present, its `x-mnemonic`
`ed25519_pubkey_base58` equals `key`, and it carries a detached JWS by `key`, as
`recipient_from_verified_card` requires. If the record has `enc_key`, the card's
`x-mnemonic` `enc_key` equals it; if not, the card has none. The owner signs the
card again after each rotation. A record that breaks these rules is invalid.

**Accounts (planned).** `KEY_BINDING_V1` (`work/dual-key-identity/`) is not built.
The field stays planned until it ships. A verifier then accepts a binding only if
its `ed25519_pubkey_base58` equals the record `key`. After a rotation, the owner
signs new bindings with the new key.

## Chain rules

A resolver applies these rules to all valid records with the same `id`. Two
records with the same payload bytes are one record.

1. **Inception.** Record `seq 0` has `prev == null`, a valid signature by `key`,
   and an `id` equal to the blake3 of its own JCS with `id` left out. A second
   `seq 0` therefore has a different `id`.
2. **Link.** Record `seq n` has `prev` equal to the payload hash of a valid record
   `seq n-1` with the same `id` (its parent).
3. **Update.** `key` equals the parent `key`, and that key signs the record.
   `next_key_hash` equals the parent `next_key_hash`. Otherwise a thief with the
   current key could point the pre-commitment at its own key.
4. **Rotation.** `blake3(32 raw bytes of key)` equals the parent `next_key_hash`,
   and the new key signs the record. The new `next_key_hash` differs from the
   hash of every key in `keys_history`.
5. **Rotation wins.** When valid records fork from one parent, a branch that
   contains a valid rotation beats each branch that contains none. The resolver
   discards the losing branches. This also defeats a fork by a retired key: the
   owner's branch from that point contains the rotation that retired it.
6. **Duplicity.** If two or more branches from one parent each contain a valid
   rotation, the identity is `conflicted` from that parent. If no branch contains
   a rotation, the identity is `conflicted` until one branch gains a rotation.
7. **Revocation.** A record with `revoked: true` ends the identity. A revocation
   by rotation (signed by the next key) is final. A revocation by update (signed
   by the current key) is provisional: under rule 5, a rotation from the same
   parent discards it. A thief with only the current key cannot end the identity.
8. **Expiry.** `valid_until` is at most 90 days after `created_at`. The signer
   sets `created_at`, so the resolver also bounds it by the record's anchor time
   (the ArDrive Turbo receipt timestamp): a record whose `created_at` is more than 10
   minutes after its anchor time is invalid. A head therefore expires at most
   90 days and 10 minutes after it was anchored, whatever `created_at` claims. A
   record with no anchor time is not counted. After the head's `valid_until`,
   the identity resolves to `expired`.
9. **Gaps.** If `seq n` is missing and a later record is signed by the head key or
   by a key whose hash equals the head `next_key_hash`, the identity resolves to
   `incomplete` with no current key. A discovery source failure or a budget limit
   also gives `incomplete`.

The resolver returns `{ status: active | expired | revoked | conflicted |
incomplete | unknown, head, keys_history }`. `keys_history` lists each key with
its `seq` range and the Arweave anchor time of the records that started and
ended its use.

**Key theft.** If the next key is stolen, the thief can rotate and take control.
The owner can then only make the identity `conflicted`. So the owner protects the
next key at least as well as the current key: an offline file, a hardware key or
a second HSM slot. If both keys are stolen, the identity is lost. That case is
out of scope.

**Old signatures.** A verifier uses the Arweave anchor time of a signed artifact
and of each record, not `created_at`, which the signer sets. A signature by a key
whose artifact is anchored after the record that retired the key is invalid. A
signature with no independent anchor time proves only that the key signed it at
some time.

## Publish

- **Path.** A new artifact type `agent-record` in the hosted ingestion adapter
  (`mcp/src/ingestion.rs`, next to `memory` and `sealed`). The server runs
  `verify_record` and checks the size limits.
- **Authorization.** If the parent is in the index, the server accepts a record
  only if rule 1, 3 or 4 authorizes its signer for that parent. It still accepts
  an authorized record that conflicts with the index, because conflicts are
  evidence. The resolver ignores unauthorized records.
- **Account.** The adapter does not require the JWT subject to equal the record
  signer, because a rotation is signed by an offline key with no account.
  Payment and free quota apply to the JWT subject. The `Producer` tag holds the
  record signer.
- **Delivery.** The existing exact-byte Arweave delivery, with tags
  `Mnemonic-Type=agent-record`, `Agent-Id=<id>` and `Agent-Seq=<seq>`. A record is
  a public write, so the existing public-write confirmation applies.
- **Stdio.** A local stdio server does not publish records in version 1. Owners
  publish through an HTTP server with the CLI or SDK. Local publish is planned.

## Resolve

- **Index.** The hosted server keeps a table of authorized records, keyed by
  `(id, seq, payload hash)`. It rebuilds the table from Arweave by the `Agent-Id`
  tag through Arweave GraphQL (`https://arweave.net/graphql`). Items count only
  after a gateway reports them in a block.
- **Endpoints.** `GET /api/agents/{id}` returns the chain and the resolved status.
  It is mounted outside the bearer-auth layer. No payment.
- **MCP tool.** `mnemonic_resolve_agent({ id })`: read-only and free, on HTTP and
  stdio. It is added to `ALLOWLIST_TOOLS_CALL_NAMES` (`mcp/src/oauth/mod.rs`).
  It returns the signed records of the chain, each with its Arweave transaction
  id and anchor time, and the server's resolved view (status, current key,
  `enc_key`, services and AgentCard). The resolved view is only a hint: the
  server is an index, not an authority, and could substitute a key or a card.
  A caller that seals to the agent or trusts its key verifies the records
  itself (SDK `resolveAgent`, CLI `show`). An MCP client without a verifier
  treats the result as unverified. No MCP tool publishes, rotates or revokes.
- **Local verification.** The SDK and CLI verify every chain with the same rules,
  whatever the source. A caller can also query the Arweave GraphQL index directly.
  That index is a second party and can also hide records.
- **Rollback.** A truncated chain still verifies. The SDK therefore keeps the
  highest verified head for each `id` and rejects a chain without that head,
  unless rule 5 discards the head. The 90-day expiry bounds how long a hidden
  update can stay unseen.

## Client surfaces

| Surface | Commands or functions | Who uses it |
|---|---|---|
| CLI | `mnemonic agent init` (first record and next key file), `publish`, `rotate --next-key-file`, `revoke`, `show <id>` | Owners and operators; the policy signer |
| SDK | `buildAgentRecord`, `signAgentRecord` (with a `Signer`), `publishAgentRecord`, `resolveAgent`, `verifyAgentChain` | Applications, including web front ends |
| MCP | `mnemonic_resolve_agent` (read-only) | Agents |
| Web app | An identity page that uses the SDK in the browser | Humans (planned after the CLI) |

## Use by sealed A2A and integrations

- A sender pins the recipient's `id`, not a key. It resolves the record, then
  builds `RecipientCard { card: agent_card, trusted_card_signer: key }` for the
  existing `recipient_from_verified_card`. A venue or a directory can list `id`
  values, but it cannot substitute keys.
- A verifier of an old memory or attestation finds the signer key in
  `keys_history` and compares anchor times (section "Chain rules").
- The Warrant swap specification uses `id` for counterparty identities and the
  `CounterpartyIn` atom.

## Planned extensions (not in version 1)

- **Endorsements.** `AGENT_ENDORSEMENT_V1`: endorser `E` vouches for `id` for a
  purpose until a time. Published and resolved the same way.
- **Control key.** A separate key for record updates, so the agent host's key
  can only sign messages.
- **ERC-8004 mirror.** `setMetadata(agentId, "mnemonic.agent_id", id)` on the
  ERC-8004 Identity Registry, and the ERC-8004 `agentId` in the record.
- **Witness receipts.** Independent servers countersign each record, so hiding a
  record needs several colluding servers.

## Tests

- Golden vectors for `AGENT_RECORD_V1`, Rust and TypeScript byte parity.
- One test per chain rule, including: a second `seq 0` (different `id`); a
  rotation by a wrong key; an update that changes `next_key_hash`; a stolen-key
  update followed by an owner rotation (rotation wins); a double update by a
  stolen current key, then an owner rotation; a fork by a retired key; two
  rotations from one parent (`conflicted`); a stolen-key revocation followed by
  an owner rotation; revocation by the next key (final); expiry; a hidden middle
  record (`incomplete`).
- Malleability: an extra unprotected header label is rejected; the same payload
  with different COSE bytes is one record.
- Cross-protocol: a `sign_cose` signature over record bytes does not verify as a
  record.
- Publish: a rotation signed by a key with no account is accepted; an
  unauthorized record for a known parent is rejected.
- Index rebuild from Arweave fixtures equals the live index.
- After a rotation, the resolve helper returns the new key, and
  `recipient_from_verified_card` fails with the new card and the old trusted key.

## Tasks

Task files: [`tasks/`](tasks/).

| # | Task | Wave | Depends on |
|---|---|---|---|
| 1 | Record schema, `sign_record`, `verify_record`; chain resolver; golden vectors | 1 | — |
| 2 | Ingestion adapter, tags, index table, `GET /api/agents/{id}`, `mnemonic_resolve_agent`, tool docs and counts | 2 | 1 |
| 3 | CLI `mnemonic agent …` and next-key handling | 2 | 1 |
| 4 | WASM and SDK functions, rollback guard | 2 | 1 |
| 5 | Sealed A2A integration and remaining docs | 3 | 2, 4 |
