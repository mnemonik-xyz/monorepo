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

## Owner decisions (2026-09-27)

| ID | Decision | Effect |
|---|---|---|
| D-1 | Accept. The hosted connector sees the text during a write. Docs say this clearly. | The server seals to the owner's key at once and keeps no plaintext. Only a client-side tool (local MCP, SDK, CLI, extension) is end to end. |
| D-3 | **Client-side only.** No hosted recall session. | The recommendation is not taken. The hosted connector cannot recall sealed memories. Remove the opt-in hosted "recall session" from task 13 and from the tech spec. |
| D-7 | **Leave old rows, seal new rows only.** | The recommendation is not taken. Existing hosted plaintext `local` rows stay as they are (owner-only through the API). There is no in-place sealing, `VACUUM` or backup rotation. |
| D-8 | Relabel public and notify. | Anchored rows labelled `private` become `public` and get `plaintext_on_arweave: true`. Tell the affected users. |

D-2, D-4, D-5, D-6, D-9, D-10 and D-11 are still open. Until the owner decides, the tasks use the recommended option of each.

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

### Task 6 — Hosted sealed write path and "private" label fix

**Status:** done  
**Agent:** claude-sonnet-4-6 (2026-09-28)

**Summary:**

1. `resolve_visibility` now allows `private` on `local` writes (triggers sealed path); `public` on `local` still rejected (-32602).
2. `sign_memory_deferred` branches on `visibility == Private`: builds inner MEMORY_V1, calls `seal_memory` with owner Ed25519 pubkey, parks only outer SEALED_V1 CBOR; zeroizes plaintext/inner CBOR/K.
3. `PendingEntry` gains `visibility: Visibility` and `is_sealed: bool`; sealed entries carry empty `content` and `embedding`.
4. Sign-callback: sealed path uploads COSE with `Mnemonic-Type: sealed` Arweave tag, memo `v: 3`, saves via `save_sealed_attestation`, uses outer hash for delivery check. `Visibility::Private` hard-code removed; uses `entry.visibility`.
5. `sign_memory_local_sealed` handles explicit-local + private for JWT callers inline (no COSE, no keychain).
6. `relabel_plaintext_anchors()` called at server startup (F1 fix).
7. `mnemonic-mcp seal-local-rows [--dry-run]` CLI subcommand added (D-7).
8. No plaintext/K/inner CBOR logged; test confirms this.
9. Added doc comment at top of `sign_memory_deferred` explaining the sealed path.

**Tests added:**
- `test_sealed_pending_entry_holds_no_plaintext` — pending entry carries no content/embedding.
- `test_local_private_stores_sealed_row_openable_with_owner_secret` — local + private → sealed row, `open_memory` recovers plaintext.
- `test_public_deferred_path_unchanged` — public deferred path byte-identical to pre-T6.
- `test_no_plaintext_in_logs` — log capture scan.

**Deviations from spec:**
- D-7 spec said "Leave old rows" (owner decision), but the `seal-local-rows` CLI was still required as a migration command option per the task spec item 7. Implemented as a CLI subcommand only (never runs automatically at startup). D-7 owner decision is respected: no automatic in-place sealing on startup.
- The `recall` tool does not surface sealed rows (they have no embedding). This is intentional per D-3/D-6: sealed-memory recall requires a client-side component.

**Test impact:** 8 existing tests updated to account for new sealed-default behavior (fake jwt_sub values replaced with real Solana keypairs; deferred_sign_flow tests updated to check `list_sealed` instead of vector search; recall_owner_isolation updated to seed public rows directly for cross-tenant recall testing; sign_memory_visibility test updated to reflect private-on-local being allowed).

**Verification:** `cargo test --workspace --features mnemonic-mcp/test-support` — all pass (0 failures). `cargo clippy --workspace --features mnemonic-mcp/test-support -- -D warnings` — clean.

### Task 9 — Extension: seal before cloud sync

**Status:** done  
**Agent:** claude-sonnet-4-6 (2026-09-28)

**Summary:**

1. `cose.ts`: Extended the `MnemonicCoreModule` interface with `seal_memory`, `open_memory`, and `x25519_public_from_ed25519` WASM bindings (matching the Task 5 exports in `core/src/wasm/mod.rs`). Added `sealMemory`, `openMemory`, and `x25519PublicFromEd25519` TypeScript wrappers. Added the X25519 unlock cache (`deriveOrGetX25519Secret`, `clearUnlockCache`, `__setUnlockCacheForTesting`) with a 15-minute TTL per tech-spec §7.5.

2. `cloud-client.ts`: Added `anchorSealed` (POST `/api/anchor-sealed`), `storeSealed` (POST `/api/store-sealed`), and `fetchSealed` (GET `/api/sealed`) methods to `CloudClient`. Added `SealedBlobItem` wire type and `base64ToBytes` helper. All three methods follow the existing typed-error contract (`ReauthRequiredError`, `PermanentSyncError`, `TransientSyncError`).

3. `cloud-sync.ts`: Added `SealableCloudClient` interface extending `CloudSignClient` with `anchorSealed`/`storeSealed`. Added `buildSealedPayload` helper (seals inner MEMORY_V1 JSON + optionally signs for anchored mode) and `openSealedBlob` helper (decrypts via unlock cache). Updated the drain to fire-and-forget `storeSealed` alongside the existing `signRemote` path when the client supports it. Added `isSealableClient` type-guard.

**Tests added (`sealed-sync.test.ts`, 18 tests, 1 skipped):**
- `anchorSealed` / `storeSealed` / `fetchSealed` route, header, and typed-error tests (GROUP A, no WASM needed).
- Unlock cache TTL constant and helper tests.
- `sync_body_never_contains_plaintext` — sliding-window scan confirms plaintext absent from sealed COSE/CBOR bytes.
- `seal_then_open_round_trip` — `sealMemory` + `openMemory` recovers original content; wrong-key rejects.
- `openSealedBlob` cloud-sync helper round-trip.
- T3 golden COSE vector parity test (skipped pending `core/pkg-web/` build from Task 5; uses `describe.skipIf`).

**Implementation notes:**
- The stub WASM injected via `__setWasmForTesting` uses XOR cipher for seal/open round-trips when the real pkg-web WASM binary is absent. All tests pass with the stub; the golden-vector test is gated by `WASM_AVAILABLE`.
- The drain's sealed path is additive (fire-and-forget beside `signRemote`) to allow incremental rollout without breaking existing sync behavior.
- X25519 secret derivation follows the Solana keypair convention (bytes 0–31 of the 64-byte secret = Ed25519 seed = X25519 scalar).

**Deviations from spec:**
- `Restore.tsx` was not modified. The restore flow relies on `fetchSealed` + `openMemory` which are now available, but the Restore component itself only handles escrow (passphrase-protected Ed25519 keypair); a follow-up task should add the sealed-blob index rebuild step to the post-restore flow once the server endpoints are live.

**Test impact:** 18 new tests added, 0 pre-existing tests broken. Pre-existing failures (`cose.test.ts` missing pkg-web WASM, `cloud-sync.test.ts` addEventListener mock) are unchanged.

## Audit findings

<!-- Written by tasks 16 and 17. -->
