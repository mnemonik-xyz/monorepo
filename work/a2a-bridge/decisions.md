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

## 2026-05-01 — Decision: Positioning lock-in — "verifiable memory for trustless agents"

The body of work in this folder + ERC-8004 follow-on commits the project to a single positioning statement:

> Mnemonic is verifiable memory for trustless agents.

Explicit consequences:

- Adjacent positions are foreclosed: general AI-agent memory (head-to-head with letta / zep / mem0 / cognee on retrieval quality), agent identity standard (head-to-head with DIDs / ERC-8004), execution attestation (head-to-head with TEEs), pure on-chain protocol. We compose with these standards; we do not compete on their primary axis.
- Story is true *only if* all four pieces ship: A2A bridge V1, ERC-8004 V1 (four paths), Phase 3α anchor pluggability, `did:mnemonic:` resolver. Any one missing collapses the pitch back to "we have a nice memory format" — competitive but not differentiated.
- Any future pull toward an adjacent position must revisit `research/positioning-trustless-agents.md` before re-pivoting.

Downstream: this decision drives the sequencing in `backlog.md`, the table reorderings in `.claude/skills/project-knowledge/references/protocol-integrations.md`, and the public roadmap rewrite in `docs/WHITEPAPER.md` §14 (new Phase 5).

---

## 2026-05-01 — Decision: ERC-8004 trigger pulls Phase 3α (anchor pluggability) forward

ERC-8004 V1 has a hard prerequisite that the SVM lock be broken **during or before** the integration work. Path-b ("ship ERC-8004 while keeping Solana SPL Memo as the only anchor") is rejected: it would deepen the SVM dependency the protocol is trying to escape (`work/mnemonic-cli/backlog.md` Phase 3) at exactly the moment we extend the anchor surface to a new chain.

Scope clarification — "Phase 3α":

- *In scope for the ERC-8004 trigger:* anchor-layer pluggability only. `core/src/storage/sqlite.rs` schema migrates from solana-specific anchor columns to a discriminated `Anchor::{Solana, Ethereum, Arweave, None}` enum. New `AnchorWriter` trait under `core/src/anchor/`. Solana SPL Memo path becomes one impl; Ethereum-via-ERC-8004-Validation-Registry becomes another. Idempotent SQLite migration; legacy rows = `Anchor::Solana(...)`.
- *Out of scope:* off-chain envelope alg-pluggability (Option B of `mnemonic-cli` Phase 2). Ed25519 stays as the off-chain signer in this stage. That work remains separately tracked under `mnemonic-cli` Phase 2 / Phase 3 Option A.

Architectural insight that makes this cheap: the ERC-8004 `validationResponse` Ethereum tx **is** the anchor for any attestation routed through Path 1 (validator-as-a-service). We do not add a new anchor backend on top of Solana; we replace the anchor for ERC-8004-routed flows with the registry call itself. ~5 dev-days for `erc8004-0`.

Cross-link: a corresponding note has been added to `work/mnemonic-cli/backlog.md` "TOP PRIORITY 2 — Crypto-flexibility" so future readers there know Phase 3 is no longer purely sequential — a strict subset is pulled forward.

---

## Audit findings (placeholder)

To be populated by code-reviewer / security-auditor / test-writer agents during their respective audit waves. Format: `### YYYY-MM-DD — <agent> — <severity>: <one-line>` then bullet body.
