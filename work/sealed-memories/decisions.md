# Decisions log: sealed-memories

Append-only. Owner decisions, task reports and audit findings go here.

---

## Fixed inputs (owner)

- **2026-09-27.** "Private" means encrypted. This applies to server-stored
  (`local`) memories and to anchored memories. Sealed mode: ciphertext on
  Arweave, `K` wrapped to the author, later grants to chosen agents.
- **2026-09-27.** Key agreement for sharing uses Diffie-Hellman (X25519 ECDH,
  RFC 7748) or a similar protocol (ECIES-style or HPKE base mode).
- **2026-09-23** (`work/presentable-mvp/plan.md:124-139`). Shared memories are
  private (ciphertext) by default. Public memories are plaintext. No
  revocation of anchored data.
- **2026-09-23** (`work/presentable-mvp/plan.md:191`). Trusted-author list:
  planned.

## Spec findings (2026-09-27)

- **F1.** Anchored rows saved as `Visibility::Private` (`mcp/src/api.rs:766-784`)
  are plaintext on Arweave (`mcp/src/api.rs:569-600`). The label is an SQL
  filter only. See D-8.
- **F2.** A hosted MCP connector sends plaintext in the tool call. The hosted
  server always sees it during the call. Only a client-side component can
  give end-to-end privacy. See D-1.
- **F3.** `core/src/encrypt.rs` seals the full plaintext once per recipient.
  It cannot grant later. Task 1 replaces it.

---

## Open owner decisions

Each item: options, recommendation. Tasks that wait are named.

### D-1. Trust wording for the hosted path (blocks task 6)

The hosted server sees plaintext during a write (F2).

- A. Accept it. Label: "Encrypted at rest. On the hosted connector the server
  sees your text while it saves it, then deletes it. For end-to-end privacy,
  use the local MCP, SDK, CLI or extension."
- B. Refuse sealed writes from hosted connectors. Only E2E clients can seal.
- **Recommendation: A.** It keeps "agents interact with ease". The approve
  page and SDK check the sealed text before signing (tech-spec §8.1 step 5).

### D-2. Source of the X25519 key (blocks task 1)

- K1. Derive from the Ed25519 identity (RFC 7748 §4.1 map). No discovery.
- K2. Separate X25519 key from the seed, published in a signed record.
- **Recommendation: K1 now, reserve an optional signed `enc_key` record (K2)
  for later rotation.** Trade-off: with K1 a stolen identity key opens every
  sealed memory (tech-spec §5.3).

### D-3. Recall of sealed memories on the hosted server (blocks task 13)

- a. Client-side only. Pure hosted connectors cannot recall sealed memories.
- d. Opt-in hosted recall session: the owner unlocks a recall key for a time
  window. The server can read during the session, in memory only.
- b. Store plaintext embeddings on the server. Leaks text by inversion.
- **Recommendation: a as the base, d as opt-in, off by default, 60-minute
  limit. Reject b.**

### D-4. Embeddings for E2E clients with no local embedder (blocks task 7)

- A. `POST /api/embed`: the server embeds one text and forgets it.
- B. Local only: in-browser or Node embedder (large download, planned).
- C. No embedding: lexical search over the local decrypted cache.
- **Recommendation: A now, B later.** Label A as "the server sees this text
  once to compute the vector".

### D-5. Where grants live (blocks task 10)

- G1. Server DB (fast discovery; server sees who shares with whom).
- G2. Arweave item (permanent; public graph unless anonymous; costs quota).
- G3. Out of band (A2A message, file, link).
- **Recommendation: G1 + G3 by default, G2 as opt-in with anonymous grants.**

### D-6. On-device `local` rows (stdio local MCP, extension) (blocks task 12)

- A. Keep plaintext on the device. The device is the key holder.
- B. Seal on the device too.
- **Recommendation: A.** The threat is the server and Arweave. The OS protects
  the device. Offer B as a setting later.

### D-7. Existing hosted `local` plaintext rows (blocks task 6)

- A. Keep them as plaintext, labelled "not encrypted".
- B. Seal in place with the owner public key, then delete the plaintext.
  Run `VACUUM` and rotate backups, because old pages and backups keep plaintext.
- C. Ask each owner to export, then delete.
- **Recommendation: B.** The server needs only public keys. Server-side
  recall of these rows stops (D-3 applies).

### D-8. Fix for the "private" label on plaintext anchors (F1) (blocks task 6)

- A. Now: relabel the rows to `public` in API and UI, add
  `plaintext_on_arweave: true` to responses, update docs. When task 6 ships,
  `private` means sealed.
- B. Now: reject `participate` writes without explicit `visibility: "public"`
  until task 6 ships. Breaks legacy clients (extension).
- **Recommendation: A, and tell affected users.** Data already on Arweave
  cannot be removed.

### D-9. Primitives (blocks task 1)

- Content: XChaCha20-Poly1305 (recommended) or AES-256-GCM.
- Wrap: HPKE base mode, suite `0x0020/0x0001/0x0003` (recommended) or custom
  ECIES (X25519 + HKDF-SHA256 + XChaCha20-Poly1305).
- Padding: 256-byte buckets (recommended).
- **Recommendation: as listed.** Reasons in tech-spec §5.1-5.2.

### D-10. Bearer links in v1 (blocks task 8)

- A. Ship with a clear warning ("anyone with this link can read it forever").
- B. Grants to known keys only.
- **Recommendation: A.** It is the only path for a reader with no key yet.

### D-11. Price of a sealed anchor (blocks task 10)

- A. Same as a public anchor (same free quota, same x402 price).
- B. Different price.
- **Recommendation: A.** Cost to the operator is almost the same (small size
  overhead).

---

## Task reports

<!-- Format per task: Status, Commit, Agent, Summary, Deviations, Reviews, Verification. -->

## Audit findings

<!-- Written by tasks 16 and 17. -->
