# Decisions — a2a-bridge

Append-only log. Each entry: date, who, what, why, what changes downstream.

---

## 2026-05-01 — Feature folder created

Author: claude (research synthesis from `work/docs-actualization/` review).

Origin: gap analysis on this branch (`claude/document-architecture-gaps-vfUxJ`) identified the missing A2A bridge as the highest-leverage of five gaps:
- Six of eleven `docs/usecases/*.md` are A2A-shaped today but have no implementation glue.
- Whitepaper §8 explicitly positions Mnemonic underneath A2A; no code matches the prose.
- Differentiation against unsigned-memory competitors (letta / zep / mem0 / cognee) is strongest along the signed-attestation × multi-agent-protocol-binding axis, which no competitor occupies.

Scope locked-in for V1: Task / Message / Artifact attestation; `contextId`-keyed recall; AgentCard `x-mnemonic` extension; sidecar + library + SDK + MCP exposure; conformance fixtures; threat model.

Out of V1: SSE per-chunk attestation, semantic recall over A2A events, chain-pluggable anchor (inherits from `mnemonic-cli` Phase 3).

---

## 2026-05-01 — Decision: dual canonicalization preserved

Do not merge JCS (RFC 8785, JSON) and deterministic CBOR. The bridge canonicalizes A2A objects via JCS and wraps the resulting bytes verbatim inside a CBOR envelope. Two canonical forms, both stable, neither modified.

Why: JCS is what A2A's own AgentCard JWS uses. Re-canonicalizing through CBOR would mean any A2A-native verifier diverges from us. Preserving JCS bytes verbatim means a verifier with only A2A tooling can still verify the inner payload and a verifier with only Mnemonic tooling can still verify the COSE envelope.

Downstream: `core/src/codec/a2a/` modules expose both `to_jcs_bytes(...)` and `to_canonical_cbor_envelope(...)`. The signing path is JCS → CBOR-envelope → blake3 → COSE_Sign1. The verifying path runs the same chain in reverse plus an independent JCS re-canonicalization check.

---

## 2026-05-01 — Decision: schema lock waits for A2A v1.0.0 GA

A2A is at v1.0.0-rc. Shipping `A2A_*_V1` schemas before GA risks frozen bytes diverging from the protocol. We ship the modules behind a cargo feature `a2a-experimental` until GA. At GA we flip the feature default and emit golden fixtures. Anyone using the experimental flag accepts schema mutability.

Downstream: `core/Cargo.toml` adds `a2a-experimental` feature. `bridge-a2a` and `mnemonic-a2a` require it pre-GA. Documentation in `references/conformance.md` flags the experimental window.

---

## 2026-05-01 — Decision: V1 does not embed A2A events

`MEMORY_V1` carries TurboQuant-compressed embeddings; `A2A_*_V1` does not. Recall on the bridge is `context_id`-keyed and time-ordered, no cosine. Users who want semantic recall over A2A content attest a parallel `MEMORY_V1` with the same `context_id`.

Why: A2A messages are mostly short and structured; embedding them adds noise and ~5x storage with low retrieval value. Composability via shared `context_id` keeps the door open without baking the cost into V1.

---

## Audit findings (placeholder)

To be populated by code-reviewer / security-auditor / test-writer agents during their respective audit waves. Format: `### YYYY-MM-DD — <agent> — <severity>: <one-line>` then bullet body.
