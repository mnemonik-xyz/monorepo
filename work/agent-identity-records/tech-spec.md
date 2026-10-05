---
created: 2026-10-05
updated: 2026-10-05
status: draft
type: feature
size: L
---

# Tech Spec: Agent identity records

Status: **planned**. Mnemonik becomes the source of agent identity: an agent's
owner publishes a self-signed record through Mnemonik, and anyone resolves and
verifies it. No party, including the Mnemonik operator, can forge or replace a
record.

## Goal

- One stable identifier per agent: the Ed25519 key that created it.
- The current signing key, encryption key, endpoints and chain accounts of that
  agent, resolvable by anyone from the identifier.
- Key rotation and revocation that a stolen current key cannot hijack.
- Old memories and attestations stay verifiable after a rotation, because the
  record chain lists every key the identity used.

Today an integration must exchange AgentCards and pin a "trusted card signer"
out of band (`docs/sealed-a2a.md`), and key rotation has no design.

## Design principles

1. **Self-certifying.** The identifier is a public key. Only that key, or a key it
   pre-committed, can extend the record chain.
2. **Mnemonik is an index, not an authority.** The operator stores and serves
   records. It can delay or hide a record. It cannot forge one. Anyone can rebuild
   the index from Arweave and verify it locally.
3. **The owner signs, never the agent.** Publishing, rotation and revocation need
   the owner's identity key. The agent (LLM) can only read.
4. **Trust is separate.** A valid record proves control of a key. Whether to deal
   with that agent is a policy decision (allowlists, endorsements, reputation).

## Record: `AGENT_RECORD_V1`

JSON in JCS form, signed with `sign_cose` by the key in `key`:

```text
{
  protocol:      "mnemonic.agent.record.v1"
  id:            base58 Ed25519 key of seq 0 (stable identifier; did:key for display)
  seq:           integer, 0 for the first record
  prev:          hex blake3 of the exact COSE bytes of record seq-1; null at seq 0
  key:           base58 Ed25519 key that signs this record
  next_key_hash: hex blake3 of the next Ed25519 public key (pre-rotation)
  enc_key:       base64 X25519 public key, optional
  agent_card:    A2A AgentCard object, optional (no JWS needed; this record signs it)
  services:      [{ type: "a2a" | "mcp" | "https", url }], optional
  controller:    id of the owner's own identity record, optional
  accounts:      [KEY_BINDING_V1 proofs (work/dual-key-identity)], optional
  created_at:    RFC 3339
  valid_until:   RFC 3339
  revoked:       bool
}
```

Limits: 16 KiB per record, 16 services, 16 accounts.

## Chain rules

A resolver applies these rules to all records with the same `id`:

1. **Inception.** Record `seq 0` has `key == id`, `prev == null`, and a valid
   signature by `id`.
2. **Link.** Record `seq n` has `prev` equal to the hash of an accepted record
   `seq n-1`.
3. **Update.** If `key` equals the key of record `n-1`, that key signs it. An
   update keeps `next_key_hash` unchanged. Otherwise a thief with the current key
   could point the pre-commitment at its own key.
4. **Rotation.** If `key` differs, `blake3(key)` must equal the `next_key_hash` of
   record `n-1`, and the new key signs the record. A thief who has only the
   current key cannot rotate.
5. **Rotation wins.** If a rotation and an update both claim `seq n` on the same
   `prev`, the rotation is accepted and the update is discarded, together with any
   record built on it. This lets the owner recover from a stolen current key: it
   rotates from the last record that it trusts.
6. **Duplicity.** Two different updates, or two different rotations, for the same
   `seq` and `prev` make the identity `conflicted` from that point. A conflicted
   identity resolves to no key. The owner starts a new identity.
7. **Revocation.** A record with `revoked: true` ends the identity. No record
   follows it. The current key can revoke through an update. The pre-committed
   next key can revoke through a rotation; rule 5 then makes it win.
8. **Expiry.** After `valid_until` of the newest record, the identity resolves to
   `expired`. The owner extends it with an update.

The resolver returns `{ status: active | expired | revoked | conflicted | unknown,
head, keys_history }`. `keys_history` lists every key with its `seq` range, so a
verifier can check an old signature against the key that was valid then.

The owner keeps the next private key away from the signing host: an offline
file, a hardware key or a second HSM slot. If the current key and the next key
are both stolen, the identity is lost. That case is out of scope.

## Publish

- **Path.** A new artifact type `agent-record` in the hosted ingestion adapter
  (`mcp/src/ingestion.rs`, next to `memory` and `sealed`). The server checks the
  COSE signature, the `protocol` value, `signer == key`, the size limits and the
  link fields. It does **not** reject a record that conflicts with its index.
  Conflicts are evidence, and the resolver handles them.
- **Delivery.** The existing exact-byte Arweave delivery, with tags
  `Mnemonic-Type=agent-record`, `Agent-Id=<id>` and `Agent-Seq=<seq>`.
- **Payment.** The existing payment and free quota rules apply. A record is a
  public write, so the existing public-write confirmation applies too.
- **Stdio.** A local server publishes through the same delivery. No local-only
  mode exists: a record that only one machine knows identifies nobody.

## Resolve

- **Index.** The hosted server keeps a table of received records, keyed by
  `(id, seq, hash)`. It rebuilds the table from Arweave by the `Agent-Id` tag,
  through the configured enumeration source (the Irys endpoint; see
  `work/DECOUPLING-SEQUENCE.md` on public gateways).
- **Endpoints.** `GET /api/agents/{id}` returns the chain and the resolved status.
  No authentication, no payment.
- **MCP tool.** `mnemonic_resolve_agent({ id })`: read-only, free, on HTTP and
  stdio. It returns the resolved status, current key, `enc_key`, services, agent
  card and accounts. No MCP tool publishes, rotates or revokes.
- **Local verification.** The SDK and CLI verify every chain locally with the
  same rules. A caller can also fetch the records straight from Arweave and skip
  the Mnemonik server.

## Client surfaces

| Surface | Commands or functions | Who uses it |
|---|---|---|
| CLI | `mnemonic agent init` (first record and next key file), `publish`, `rotate --next-key-file`, `revoke`, `show <id>` | Owners and operators; the policy signer |
| SDK | `buildAgentRecord`, `signAgentRecord` (with a `Signer`), `publishAgentRecord`, `resolveAgent`, `verifyAgentChain` | Applications, including web front ends |
| MCP | `mnemonic_resolve_agent` (read-only) | Agents |
| Web app | An identity page that uses the SDK in the browser | Humans (planned after the CLI) |

## Use by sealed A2A and integrations

- A sender pins the recipient's `id`, not a key. It resolves the record, then
  builds `RecipientCard { card: agent_card, trusted_card_signer: current key }`
  for the existing `recipient_from_verified_card`. A venue or a directory can list
  `id` values, but it cannot substitute keys.
- A verifier of an old memory or attestation finds the signer key in
  `keys_history` and checks the time of the signature against that key's range.
- The Warrant swap specification uses `id` for counterparty identities and the
  `CounterpartyIn` atom.

## Planned extensions (not in version 1)

- **Endorsements.** `AGENT_ENDORSEMENT_V1`: endorser `E` vouches for `id` for a
  purpose until a time. Published and resolved the same way.
- **ERC-8004 mirror.** `setMetadata(agentId, "mnemonic.agent_id", id)` on the
  ERC-8004 Identity Registry, and the ERC-8004 `agentId` in `accounts`. This links
  the two registries in both directions.
- **Witness receipts.** Independent servers countersign each record, so hiding a
  record needs several colluding servers.

## Tests

- Golden vectors for `AGENT_RECORD_V1`, Rust and TypeScript byte parity.
- One test per chain rule, including: a rotation by a wrong key; an update that
  changes `next_key_hash`; a stolen-key update followed by an owner rotation (the
  rotation wins); two conflicting updates (`conflicted`); revocation by the next
  key; expiry; a missing `seq` (resolves to the last contiguous head).
- Index rebuild from Arweave fixtures equals the live index.
- `recipient_from_verified_card` succeeds with a resolved record and fails after
  a rotation that the sender has not resolved.

## Tasks

Task files: [`tasks/`](tasks/).

| # | Task | Wave | Depends on |
|---|---|---|---|
| 1 | Record schema, sign, verify; chain resolver; golden vectors | 1 | — |
| 2 | Ingestion adapter, Arweave tags, index table, `GET /api/agents/{id}`, `mnemonic_resolve_agent` | 2 | 1 |
| 3 | CLI `mnemonic agent …` and next-key handling | 2 | 1 |
| 4 | WASM and SDK functions | 2 | 1 |
| 5 | Sealed A2A integration, docs (`docs/tools.md`, `docs/sealed-a2a.md`, `AGENTS.md`, `CLAUDE.md` tool count) | 3 | 2, 4 |
