---
created: 2026-07-01
type: architecture-layers
status: draft
assumption: universal-cheap-zkVM (zigz); no Job-A/Job-B split; no in-guest-hashing constraint
grounds: work/research/computation-proof/{architecture,tech-spec,v1-agentic-payments}.md, work/research/protocol/design.md, core/ mcp/ prover/ packages/ webapp/
---

# Mnemonic — Layered Architecture

**One sentence.** Mnemonic produces and verifies *self-verifying intent→action objects*: signed, content-addressed records where the only things a relying party must trust are **the principal's signature** and **proof soundness** — everything else (agent, prover honesty, storage, anchor) is untrusted and re-checkable.

**Exercise assumption.** The zkVM (`zigz`) is treated as a **universal prover**: it proves any bounded deterministic policy cheaply, including any in-guest hashing/recursion. So this document **drops the Job-A/Job-B split** and the "no in-guest hashing" division-of-labor rule from `architecture.md §2/§4`. The Rust-does-hashing / guest-does-arithmetic split becomes an *optimization*, not an architectural boundary. The `core/ = verify-only`, `prover/ = produce`, one-way `core→mcp` dependency invariants are **unaffected** and still hold.

---

## 1. Layer stack (top → bottom)

Each layer: **responsibility · real components · role (Produce/Verify/Orchestrate/Store/Anchor) · trust level.**

| # | Layer | Responsibility | Real components | Role | Trust |
|---|---|---|---|---|---|
| L0 | **Client / Presentation** | UX to sign, recall, prove, verify; hold user keys non-custodially; trigger produce; verify locally | `webapp/` (React SSR: `pages/Sign.tsx`, `Ledger.tsx`, `Consent.tsx`, `IdentityPanel.tsx`; `lib/wasm.ts`, `lib/api.ts`), `packages/extension/` (MV3 service worker), `packages/cli/` (`commands/{sign,verify,prove,identity}.ts`) | Verify (+ trigger Produce) | **Untrusted edge**; holds keys; verifies locally |
| L1 | **SDK / Edge crypto** | Envelope build + **wasm verify** at the edge; no secrets to server; Ed25519/COSE; content-hash | `packages/sdk/` (`client.ts`, `signer.ts` `LocalSigner`, `cose.ts`, `keypair.ts`, `wasm.ts`/`wasm.browser.ts`), `core/src/wasm/mod.rs` (wasm-bindgen: `generate_keypair`, `sign_cose_payload`, `blake3_hash`, `to_canonical_cbor_bytes`, `sign_challenge`) | Verify / Produce-envelope | No secrets, no server trust |
| L2 | **Application / Orchestration** | Drive produce→bind→anchor and load→verify; auth (OAuth), pending-sign relay, recall; **signs nothing** | `mcp/` server (`mcp.rs`, `tools.rs`, `trajectory_tools.rs`, `pending.rs`, `publish.rs`, `oauth/`, `api.rs`, `seed.rs`), `packages/mcp/` (stdio bridge) | Orchestrate | Non-custodial relay; **no authority** (design.md Roles) |
| L3 | **Domain / Correspondence** | The moat: `verify_correspondence` 5-checks; compute `action_commitment` binding; COSE verify; **verify-only, never signs** | `core/src/correspondence/mod.rs` (`verify_correspondence`, `action_commitment`, `PolicyCertificate`, `CorrespondenceVerifier`, `MockVerifier`), `core/src/codec/{sign,canonical,hash,schema}.rs` | **Verify** | Trustless to run (differential-tested) |
| L4 | **Proving** | PRODUCE the correspondence proof π + collect evidence; run the policy guest | `prover/` (`mnemonic-prover`: `EvidenceSource`+`StubEvidence`, `Prover`+`MockProver`; *designed:* `prove/zigz.rs` `ZigzProver`, `evidence/tlsn.rs`, `guests/payment_mandate_v1/`) | **Produce** | Trusted for **soundness only** (unaudited zigz → experimental) |
| L5 | **Identity / Keys** | Ed25519 keypair custody; `did:sol`/`did:key`; OS-keychain vs file; sign/verify primitives | `core/src/identity/` (`mod.rs`, `keystore_os.rs`, `keystore_file.rs`, `ensure.rs`, `token_store.rs`), `packages/{sdk,cli}` keypair/localStorage | Produce (sign) / Verify (sig) | Principal & agent keys = **root of authority**; per-key compromise out-of-proof-scope |
| L6 | **Policy Registry** | Map `policy_id (= program_hash)` → audited guest ELF + `params_schema`; publish + anchor entry | *designed only* — see `architecture.md §8`. Guest source would live `prover/guests/<policy>/`; `policy_id` referenced today in `INTENT_V1.constraints` | Store / Verify (`program_hash == intent.policy_id`) | Registry serves content-addressed program; **audit** of the policy is the real trust root |
| L7 | **Data / Storage** | Hold bytes, content-addressed, retrievable; hot cache + durable custody; recall index | `core/src/storage/` (`traits.rs` `AttestationStore`/`LineageStore`, `sqlite.rs`, `mode.rs`, `trajectory_arweave.rs`, `trajectory_sqlite.rs`), `core/src/arweave/mod.rs` (Irys ANS-104), `core/src/embed/mod.rs` (`Embedder`) | **Store** | Untrusted (content-addressed); availability = durability class |
| L8 | **Anchor** | Batched Merkle root on a clock — existence + time + order; **never holds data** | `core/src/solana/mod.rs` (SPL Memo, current), `core/src/merkle.rs` (`trajectory_root`, `commitment_root`, `prove`/`verify`); *designed:* `Anchor` trait + OTS→Bitcoin / RFC-3161 backends | **Anchor** | Neutral, untrusted (root only) |

**Layer-stack invariants (survive the universal-prover assumption):**
- **Verify everywhere.** L3 is a *library, not a place* (`design.md`): the same pure-Rust `verify_correspondence` runs in browser (L1 wasm), CLI (L0), and any auditor/contract. No "verifier node."
- **Produce is isolated.** Only L4 (`prover/`) produces π; L3 (`core/`) only verifies; L2 (`mcp/`) only orchestrates and **signs nothing** (non-custodial guard is a test).
- **One-way dependency DAG** (`architecture.md §6`): `core → {prover, wasm, native, mcp}`, `prover → mcp`, `native → mcp`, `wasm → sdk`. Everything points at the portable `core`.

---

## 2. Component inventory

| Component | Layer | Language / crate | Responsibility | Depends on |
|---|---|---|---|---|
| `webapp` | L0 | TS/React (Vite SSR) | Sign/recall/verify UI; holds localStorage keypair; wasm verify | `sdk`, `core/wasm` |
| `packages/extension` | L0 | TS (MV3) | Client-side keys + local recall; same crypto pipeline (byte-parity) | `core/wasm` golden fixtures |
| `packages/cli` | L0 | TS (Node) | `sign` / `verify` / `prove` (identity challenge) / `identity`; keychain | `sdk` |
| `packages/sdk` (`@mnemonik-xyz/sdk`) | L1 | TS | Envelope build, COSE, `LocalSigner`, wasm verifier loader, OAuth | `core/wasm` |
| `core/src/wasm` | L1 | Rust→wasm (`wasm-bindgen`) | Thin crypto wrappers: keygen, COSE sign, canonical CBOR, blake3 | `core/{codec,identity,compress}` |
| `mcp` (`mnemonic-mcp`) | L2 | Rust (axum) | Orchestrate sign/store/anchor/recall; OAuth; pending-sign; seed | `core` (native), *designed:* `prover` |
| `packages/mcp` | L2 | TS | stdio↔http bridge, binary install/doctor | — |
| `core::correspondence` | L3 | Rust (`mnemonic-core`) | `verify_correspondence` (5 checks), `action_commitment`, `PolicyCertificate` | `codec`, `merkle` |
| `core::codec` | L3 | Rust | Canonical CBOR, blake3 hash, COSE_Sign1 sign/verify, schema registry | serde, ciborium, coset |
| `core::trajectory` | L3 | Rust (gated) | Chain integrity + verdict coverage over agent steps (adjacent domain) | `codec`, `merkle` |
| `core::merkle` | L3/L8 | Rust | Order-preserving Merkle root + inclusion proofs (`trajectory_root`, `prove`, `verify`) | blake3 |
| `prover` (`mnemonic-prover`) | L4 | Rust (gated) | `EvidenceSource`/`Prover` traits; `StubEvidence`+`MockProver`; *(zigz Wave 2)* | `core::correspondence` |
| `core::identity` | L5 | Rust | Keypair custody, `did:sol`/`did:key`, OS-keychain/file keystore, tokens | solana-sdk |
| policy registry | L6 | *unbuilt* | `policy_id`→ELF + `params_schema`, anchored entry | `merkle`, anchor |
| `core::storage` | L7 | Rust | `AttestationStore`/`LineageStore`; SQLite recall cache; Arweave trajectory store | rusqlite, `arweave` |
| `core::arweave` | L7 | Rust | Irys ANS-104 bundle upload (D3), arlocal dev stub | reqwest, solana-sdk |
| `core::embed` | L7 | Rust | `Embedder` trait (fastembed/OpenAI); TurboQuant compressed vectors | fastembed / reqwest |
| `core::solana` | L8 | Rust | SPL Memo write/read (current anchor backend) | solana-sdk |
| `Anchor` trait + OTS/TSA | L8 | *unbuilt* | Pluggable batched-root anchoring | `merkle` |

---

## 3. Data layers / data model

Objects are **signed COSE_Sign1 + content-addressed by blake3** (`design.md`). Placement columns: **Hot** = local SQLite cache (`core/src/storage/sqlite.rs`), **Durable** = D1–D3 custody (Arweave/Filecoin/relay), **Anchor** = only a 32-byte batched root on-chain.

### Identity data
| Object | Fields | Where | Addressing | Lifecycle |
|---|---|---|---|---|
| **Keypair / Identity** | Ed25519 secret (64B), `pubkey_base58`, `did:sol`/`did:key`, `created_at`, `IdentityStorage` (OsKeychain\|File) | OS keychain / `~/.mnemonic/identity.json` / browser localStorage — **never leaves client** | `did:sol:<pubkey>` | Created at bootstrap (`identity::ensure`); root of authorship for every signature |
| **OAuth token** | JWT (`sub`=owner pubkey), refresh | `token_store.rs`; server session | — | OAuth flow; scopes recall tenancy |

### Policy data
| Object | Fields | Where | Addressing | Lifecycle |
|---|---|---|---|---|
| **PolicyCertificate** (`core::correspondence`) | `intent_hash, action_commitment, evidence_commitment, policy_id, params_hash, public_inputs[], proof_kind ("zigz"\|"snark"\|"zktls"\|"mock"), proof_ref, backend` | Nested in `ACTION_V1.metadata.correspondence` (Hot+Durable) | `proof_ref = blake3(π)` | Produced by L4; bound into Action; re-checked by L3 |
| **Proof bytes π** | zigz proof (~7–40 KB) | **Durable** (Arweave); referenced by hash | `proof_ref` | Off-envelope; fetched at verify |
| **Registry entry** *(designed)* | `name, policy_id(=program_hash), params_schema, version, publisher_sig` | Durable + Anchor | `program_hash = blake3(ELF)` | Author→reproducible build→audit→publish→anchor (`architecture.md §8`) |

### Memory / knowledge data
| Object | Fields | Where | Addressing | Lifecycle |
|---|---|---|---|---|
| **Memory** (`MEMORY_V1`) | `artifact_id, type:"memory", schema_version, content, producer, created_at, metadata{embedding_compressed, embed_dim, ...}, tags` | Hot (SQLite + TurboQuant vector) + optional Durable | `content_hash = blake3(canonical_cbor)` | Signed client-side; recall via cosine (`AttestationStore::search`); "SQLite is a rebuildable cache" (`core::rebuild`) |
| **Trajectory Step / Verdict / Summary** (`trajectory-experimental`) | `step`: seq, prev_hash, content_hash, signature; `verdict`: judge≠producer, status, proof_ref | Hot + Durable (Arweave bundle) | blake3; `trajectory_root` (order-preserving Merkle) | Hash-linked chain; independent judge coverage; `knowledge_refs` bind memories into an Action |

### Action / correspondence data
| Object | Fields | Where | Addressing | Lifecycle |
|---|---|---|---|---|
| **Intent** (`INTENT_V1`) | required `artifact_id, type:"intent", schema_version, constraints, producer(=principal did:sol), created_at`; optional `expiry, nonce, metadata, tags`. `constraints` = AP2 mandate (cap, currency, allowlist roots, `policy_id`, params hash) | Durable (+ Hot); `intent_hash` shared out-of-band | `intent_hash = blake3(canonical_cbor)` | **Principal signs** (root of authority); immutable |
| **Action** (`ACTION_V1`) | required `artifact_id, type:"action", schema_version, content, producer(=agent), created_at, intent_ref`; optional `knowledge_refs[], metadata.correspondence(=cert), tags` | Durable (audit-grade D2/D3) + Hot | `action_commitment = blake3(canonical_cbor of content,producer,created_at,intent_ref,knowledge_refs)` — **excludes metadata** (circularity fix, `tech-spec.md`) | **Agent signs**; `intent_ref == intent.content_hash` |
| **Evidence** (`prover::Evidence`) | `commitment, proof_ref` (+ raw attestation bytes / zkTLS transcript) | Durable | `evidence_commitment` | Collected by `EvidenceSource`; binds action fields to merchant-authenticated reality (clause 5, `v1-agentic-payments.md`) |

### Storage / anchor metadata
| Object | Fields | Where | Addressing | Lifecycle |
|---|---|---|---|---|
| **Bundle** | ordered objects; Merkle root | Durable | `trajectory_root(ordered hashes)` | One root amortizes anchor cost ~1000× |
| **Anchor receipt** *(trait designed)* | `root, AnchoredTime`, backend proof (OTS/Solana tx/TSA) | Anchor chain | root | Proves root existed before T |
| **AttestationRow** (`storage::traits`) | `attestation_id, content, content_hash, solana_tx, arweave_tx, signer_pubkey, owner_pubkey, write_mode(Local\|Participate), visibility(Private\|Public), embedding` | **Hot** (SQLite) | `content_hash` | Rebuildable recall cache; tenant-scoped by `owner_pubkey` |
| **Durability commitment** *(designed)* | class `D0`(dev)\|`D1`(k relay receipts)\|`D2`(Filecoin PoSt)\|`D3`(Arweave) | part of the object | — | Verifier checks class ≥ its bar; **D0 invalid for audit** |

---

## 4. Data flow

### (a) Produce + anchor an attested action

```
L0 Principal (webapp/cli)
  |- L1/L5 sign INTENT_V1 (Ed25519/COSE, client-side wasm) -> intent_hash
                                                  | (shared out-of-band)
L0 Agent decides an action, builds ACTION_V1 (content, intent_ref=intent_hash, knowledge_refs)
  |- L3 core::action_commitment(action)  <- blake3 over pre-cert fields (excludes metadata)
  |- L2 mcp orchestrates:
        L4 EvidenceSource::collect(action_commitment) -> Evidence{commitment, proof_ref}   [Durable]
        L4 Prover::prove(intent_hash, action_commitment, evidence, policy_id) -> PolicyCertificate
             . public_inputs bind (intent_hash, action_commitment, evidence_commitment)
             . pi (proof bytes) -> Durable (Arweave); cert.proof_ref = blake3(pi)
        L3 bind cert into ACTION_V1.metadata.correspondence
  |- L0/L5 Agent signs the full ACTION_V1 (COSE) — no fixed-point (cert added before signing)
  |- L7 store bytes (D2/D3 for the verifiable core; Hot SQLite cache row)
  |- L8 batch -> Merkle root -> anchor (Solana memo now; OTS->Bitcoin default target)
```

Created where: `intent_hash` at L1 (principal client). `action_commitment` at L3. `Evidence`/`Certificate`/π at L4. Binding at L3. Signature at L0/L5. Storage at L7. Root at L8. **Server (L2) signs nothing.**

### (b) Verify

```
Any Verifier (auditor / counterparty / browser / contract) — trustless, no re-execution, no witness
  |- L7 fetch INTENT + ACTION by content_hash; fetch pi by cert.proof_ref
  |- L3 core::verify_correspondence(intent_cose, action_cose, cert, verifier) -> 5 tri-states:
        1 intent_sig   — COSE over INTENT valid (verify_artifact)
        2 action_sig   — COSE over ACTION valid
        3 intent_link  — action.intent_ref == intent.content_hash == cert.intent_hash; not expired
        4 correspondence_proof — RECOMPUTE action_commitment, must == cert value AND appear in
                                 public_inputs, THEN verifier.verify_proof(cert) (re-verify pi)
        5 evidence_proof — verifier.verify_evidence(cert) (re-verify zkTLS/evidence)
  |- L6 (implied by check 4) program_hash == intent.policy_id — cannot swap a weaker policy
  |- L8 (optional) resolve anchor receipt -> existence + time; check durability class >= bar
  -> safe = all Some(true);  policy_valid = correspondence_proof
```

The **binding recompute (check 4) is enforced by `core` itself**, not the pluggable backend — so a tampered `action_commitment` fails even under `MockVerifier` (proven by `tampered_action_commitment_fails_binding`). The universal-prover assumption changes *what the guest can cheaply prove*, not this verify shape.

---

## 5. Cross-cutting concerns

**Trust boundaries.**
- **Untrusted edge (L0/L1):** holds keys, verifies locally, never trusts the server.
- **Adversarial agent (L0):** runs arbitrary code; its reasoning is *not* proven — only that the *recorded* action satisfies the policy given evidence. Design premise, not an edge case.
- **Non-custodial operator (L2/L3/L4):** produces + orchestrates; **signs nothing**; trusted only for prover *soundness*.
- **Neutral infra (L7/L8):** content-addressed + durability-guaranteed; holds no authority.

**Where crypto happens.**
| Op | Where | Component |
|---|---|---|
| Hashing (blake3, content-addressing) | L1 (wasm) + L3 (native) — same bytes | `core::codec::hash`, `wasm::blake3_hash` |
| Signing (Ed25519/COSE) | **client only** (L0/L1/L5) | `wasm::sign_cose_payload`, `sdk LocalSigner`, `codec::sign` |
| Proving (π) | L4 only | `prover::Prover` (`MockProver` → `ZigzProver`) |
| Verifying (COSE + π + evidence + binding) | L3, runs everywhere | `verify_correspondence`, `CorrespondenceVerifier` |
| Merkle root / inclusion | L8/L3 | `core::merkle` |
| Anchoring | L8 | `core::solana` (→ `Anchor` trait) |

**Client-side vs server-side vs neutral.** Signing + local verify = **client** (never server). Produce/orchestrate = **server (signs nothing)**. Storage/anchor = **neutral/untrusted**. The verifier is a **library, not a node**.

**Extensibility seams (traits).** The architecture is a set of swap points, each isolating an unbuilt/experimental piece behind a stable interface:
- `EvidenceSource` (`prover`) — `StubEvidence` → `TlsNotaryEvidence` (zkTLS).
- `Prover` (`prover`) — `MockProver` → `ZigzProver`. *Under the universal-prover assumption this is where the whole policy space lives — the guest is unconstrained.*
- `CorrespondenceVerifier` (`core`) — `MockVerifier` → pure-Rust zigz re-verifier (wasm-compilable; differential-tested vs the Zig prover).
- `AttestationStore` / `LineageStore` / `TrajectoryStore` (`core::storage`) — SQLite (Hot) / Arweave (Durable).
- `Embedder` (`core::embed`) — fastembed / OpenAI.
- `Anchor` *(designed)* — Solana / OTS→Bitcoin / RFC-3161 / none.

---

## 6. Gaps — designed but not yet built

Flagged against the code as it stands (`0a9ae36` Wave-1 mocks are the current frontier).

| Gap | Status | Where it should land | Note |
|---|---|---|---|
| **Real zigz prover** (`ZigzProver`) | **Not built** — only `MockProver` in `prover/src/prover.rs` | `prover/src/prove/zigz.rs` + `guests/payment_mandate_v1/` | The universal-prover assumption presumes this exists; today π is a stub hash |
| **Pure-Rust zigz verifier** | **Not built** — only `MockVerifier` | `core/src/correspondence/zigz.rs` (`corr-zigz`) | Requires frozen `zigz-proof-v1` format + CI differential conformance vectors |
| **zkTLS evidence** (`TlsNotaryEvidence`) | **Not built** — only `StubEvidence` (dev-only trust hole) | `prover/src/evidence/tlsn.rs` | Clause-5 binding is only as real as the evidence; operational risk concentrated here |
| **Correspondence MCP tools** | **Not wired** — `mnemonic_prove/verify_correspondence` absent from `mcp/src/tools.rs` | `mcp/src/tools.rs` | L2 does not yet orchestrate produce→bind→anchor; CLI `prove` today is *identity-challenge* signing, `verify` is *attestation-integrity* — not the 5-check path |
| **Policy Registry (L6)** | **Not built** — `policy_id` referenced in `INTENT_V1.constraints`, but no registry, `params_schema`, reproducible-build pipeline, or `publisher_sig` | new module | Trust root of the whole system; needs pinned reproducible build so `program_hash` is re-derivable |
| **Anchor trait + neutral backends** | **Not built** — only concrete `core::solana` SPL Memo | `core/src/anchor/` | `design.md` demotes Solana to one backend; the trait is the seam |
| **Durability service (D1–D3)** | **Partial** — Arweave (D3) exists; no `Store` trait unifying classes, no D1 relay receipts, no D2 Filecoin, no verifiable durability commitment | `core/src/storage` + object schema | `D0 invalid for audit`; verifier can't yet check a durability class |
| **Knowledge link binding** (Wave 4) | **Not built** — `knowledge_refs` is an optional `ACTION_V1` field only | `prover` witness + `core` verify | Bind retrieved memory hashes into the proof witness/commitment |
| **In-browser proof verify (wasm)** | **Partial** — wasm exports crypto primitives + COSE; the correspondence verifier isn't yet compiled to wasm | `core/src/wasm` once `corr-zigz` lands | "Verify everywhere" for the *proof* (not just signatures) is still pending |

### Critical files for implementation
`core/src/correspondence/mod.rs` · `prover/src/prover.rs` · `core/src/codec/schema.rs` · `mcp/src/tools.rs` · `core/src/storage/traits.rs`
