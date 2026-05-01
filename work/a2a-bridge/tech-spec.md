---
created: 2026-05-01
status: draft
size: L
branch: feat/a2a-bridge
---

# Tech Spec: A2A Bridge

## Solution

Three layers, deployable independently:

1. **Schema layer (`mnemonic-core`)** — three new CBOR schemas (`A2A_TASK_V1`, `A2A_MESSAGE_V1`, `A2A_ARTIFACT_V1`) registered in `core/src/codec/schema.rs`. Each is canonicalized through the existing `to_canonical_cbor` path, blake3-hashed, COSE_Sign1-signed via `codec::sign::sign_artifact`. Lineage links (`prev_id`) carry intra-context ordering. Zero new crypto primitives — all reuse.

2. **Adapter layer (new crate `mnemonic-a2a`)** — pure-Rust, depends on `mnemonic-core` only. Surface: `attest_message(msg, ctx) -> AttestationId`, `attest_task(task, ctx) -> AttestationId`, `attest_artifact(art, ctx) -> AttestationId`, `recall_by_context(ctx, query?) -> Vec<Attestation>`. No network, no I/O beyond `core`'s `AttestationStore`. Stateless functions; storage handle injected.

3. **Surface layer (existing `mcp/` + new `bridge-a2a/` + SDK)** — three deployment shapes consume the adapter:
   - **MCP tools** (`mnemonic_attest_a2a`, `mnemonic_recall_a2a`) added to `mcp/src/tools.rs` + `mcp/src/mcp.rs`.
   - **Reference sidecar** (`bridge-a2a/`) — axum middleware, intercepts A2A JSON-RPC, attests via the adapter, returns the attestation id in the `x-mnemonic` extension on the A2A response.
   - **SDK helpers** in `@mnemonik-xyz/sdk` — `attestA2ATask`, `attestA2AArtifact`, `recallA2AContext`, calling the MCP tools over HTTP.

## Architecture

### Files added

- `core/src/codec/schema.rs` — three new schema constants (extension; do not mutate `MEMORY_V1`).
- `core/src/codec/a2a/` — new submodule: `task.rs`, `message.rs`, `artifact.rs`, `mod.rs`. Type-safe Rust mirrors of the A2A wire types, plus `to_canonical_cbor_bytes` helpers.
- `core/tests/a2a_golden_fixtures.rs` — emitter for `{a2a_json, canonical_cbor_hex, cose_envelope_hex}` triples (gated by `golden-fixtures` feature, mirrors existing pattern).
- `bridge-a2a/` — new workspace member, `Cargo.toml` + `src/{main.rs, middleware.rs, agentcard.rs, config.rs}`. Native-only, axum-based.
- `mnemonic-a2a/` — new workspace member, `Cargo.toml` + `src/{lib.rs, attest.rs, recall.rs}`. Pure library, no async runtime requirement (storage Mutex pattern follows CLAUDE.md rule).
- `packages/sdk/src/a2a.ts` — three thin TS helpers; tests in `packages/sdk/test/a2a.test.ts`.
- `.claude/skills/project-knowledge/references/threat-model.md` — first version, A2A-boundary-focused.
- `.claude/skills/project-knowledge/references/conformance.md` — companion doc describing the published fixtures.

### Files modified

- `Cargo.toml` (workspace root) — add `mnemonic-a2a`, `bridge-a2a` to members.
- `core/src/codec/mod.rs` — re-export new submodule under `pub mod a2a`.
- `mcp/src/tools.rs` — two new tool definitions.
- `mcp/src/mcp.rs` — dispatch arms for `mnemonic_attest_a2a` and `mnemonic_recall_a2a`.
- `core/src/storage/sqlite.rs` — additive: `context_id TEXT NULL` column on `attestations` (idempotent migration), index on `(context_id, created_at)` for `recall_by_context`.
- `docs/usecases/*.md` (the six A2A-shaped ones) — append "Reference implementation" section pointing to bridge-a2a.

### Architectural rules (preserve)

- `mnemonic-a2a` and `bridge-a2a` depend on `mnemonic-core` only. Same one-way graph as `mcp/`.
- No payment / pricing logic in either new crate. Attestation cost (if any) flows through the existing `mcp/src/payment.rs` when the call enters via the MCP tool path.
- No new direct embedder providers — A2A_*_V1 attestations do **not** require an embedder (no semantic recall over A2A events in V1; recall is keyed by `context_id`, not by cosine).
- `rusqlite::Connection` lock discipline unchanged — never hold across `.await`.
- Schema bytes are immutable post-V1. Breaking change → V2.

## Decisions

### Decision 1 — Two canonicalization formats, one attestation envelope

A2A AgentCard signing uses RFC 8785 (JCS) over JSON. Mnemonic uses deterministic CBOR. We do NOT merge them. Instead the bridge canonicalizes the A2A object via JCS, then wraps the canonical JSON bytes verbatim inside a CBOR envelope (`A2A_TASK_V1` = `{schema, jcs_bytes, agent_pubkey, ts, prev_id?}`). The CBOR is canonicalized and COSE-signed; the inner JCS-bytes are byte-for-byte the same A2A canonical form. Verification: re-canonicalize A2A object via JCS → compare bytes → check COSE signature. Two independent canonicalizations, neither modified.

### Decision 2 — `contextId` is the primary index, not `messageId`

A2A's discriminator for "a multi-turn collaboration" is `contextId`. We index attestations by it and expose `recall_by_context`. `messageId` and `taskId` are secondary keys.

### Decision 3 — AgentCard binding via `x-mnemonic` extension

A2A has a first-class `Extension` mechanism. We define a single extension descriptor publishing `{ ed25519_pubkey_base58, attestation_endpoint?, conformance_version }`. No DID. No new identity crypto. Verifier resolves AgentCard → extracts `x-mnemonic.ed25519_pubkey_base58` → uses it directly with `core::codec::sign::verify_artifact`.

### Decision 4 — Sidecar AND library, not one or the other

Sidecar maximizes adoption (zero agent-side changes). Library minimizes latency and gives full control. Both reuse `mnemonic-a2a`. Sidecar is a single binary using the library; library users skip the network hop.

### Decision 5 — Schema lock against A2A v1.0.0 GA only

A2A is at v1.0.0-rc. We ship V1 schemas as `experimental` flag-gated until A2A GA. On GA: flip the flag, freeze the bytes, publish conformance fixtures. Any A2A breaking change after that → `*_V2` schemas, V1 stays valid for replay.

### Decision 6 — V1 does not embed-and-recall A2A events semantically

`MEMORY_V1` carries TurboQuant-compressed embeddings for semantic recall. A2A events do not — recall is by `context_id`, ordered by time + lineage. Reasoning: (a) A2A messages are often short and structured (tool calls, status updates); cosine search adds noise. (b) Embedding every A2A event would explode storage. (c) Users who want semantic recall of A2A content can attest a `MEMORY_V1` *alongside* the A2A_*_V1 — same `context_id`, different schema. Composable.

### Decision 7 — Conformance fixtures published as a separate npm artifact

`@mnemonik-xyz/conformance` — JSON file with golden vectors, no runtime code. Any third-party implementation (Go, Python, Java, Wasm-in-browser) can load it and prove byte-for-byte parity. Published from CI on every tagged release.

### Decision 8 — Streaming attestation deferred

A2A's `SendStreamingMessage` and `SubscribeToTask` produce SSE chunks. V1 attests only the final task state. Per-chunk attestation = future feature (`A2A_STREAM_CHUNK_V1`), goes in backlog.

## Testing

- **Unit:** each schema module has round-trip tests (encode → decode → assert equal) and canonicalization stability tests (two encodes of same input → identical bytes).
- **Property:** proptest for `attest_task` ensuring any valid A2A Task JSON produces a verifiable attestation.
- **Integration:** end-to-end test under `bridge-a2a/tests/` — start a mock A2A server, run sidecar in front, call `SendMessage`, assert response carries `x-mnemonic.attestation_id`, fetch via MCP `mnemonic_verify`, assert valid.
- **Conformance:** `golden-fixtures` feature emits 20+ vectors covering text/file/data parts, single + multi-turn, completed + failed tasks. SDK test loads the published JSON and asserts JS-side canonicalization matches.
- **Threat-model regression:** explicit tests for the cases enumerated in `references/threat-model.md` (replay, identity-substitution, JCS-CBOR canonicalization mismatch, contextId forking).

## Sequencing (waves)

- **Wave 1 — schemas** (foundation; nothing else compiles without): tasks 1, 2.
- **Wave 2 — adapter + identity binding** (parallel; different files): tasks 3, 4.
- **Wave 3 — surface** (parallel; different files): tasks 5 (MCP), 6 (sidecar), 7 (SDK).
- **Wave 4 — conformance + audit** (parallel): tasks 8.

Common conflict points: `Cargo.toml` (root), `core/src/codec/mod.rs`, `mcp/src/mcp.rs`. Schedule those touches inside single tasks.

## Risks

- **A2A pre-GA churn** — V1 schema lock would create migration debt if A2A renames a Task field. Mitigation: ship behind `experimental` feature flag until A2A GA; final schema bytes only frozen at flip.
- **JCS implementation in Rust** — the canonical-json crate ecosystem is thinner than serde_json. Mitigation: vet the crate during Wave 1 (`serde_jcs` is the leading candidate); fallback is a small in-house implementation since JCS is ~150 LOC.
- **Identity binding ambiguity** — if AgentCard has both a `signatures[]` JWS and our `x-mnemonic.ed25519_pubkey_base58`, which is authoritative? Decision: JWS verifies the AgentCard itself; `x-mnemonic` declares the Mnemonic attestation key; both must be present and the JWS must cover the extension payload. Tested explicitly.
- **Adoption tied to A2A adoption** — if A2A flatlines, bridge ROI drops. Mitigation: same adapter pattern is reusable for ACP / MCP-to-MCP delegation (see `backlog.md`).
