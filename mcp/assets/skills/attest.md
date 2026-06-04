# mnemonik-attest

## Purpose

Persist a verifiable, signed memory of a consequential decision or outcome so future agents (and the user) can recall it with cryptographic provenance. Use after something is **decided and stable**, not while it is being explored.

## Trigger

This skill is load-bearing for adoption: over-attesting pollutes the user's memory pool with scratch, under-attesting defeats the protocol. The boundary is consequentiality plus stability — has something concrete happened that another agent or the user will benefit from recalling later.

**Positive examples (DO attest):**

- A code change has been committed (commit landed on a branch, not just staged in an editor) and the rationale or constraint that drove the choice is non-obvious from the diff alone.
- A design decision was recorded — e.g., "we picked SQLite over Postgres for the local store because the protocol must work offline."
- A research conclusion was reached — e.g., "the third-party library X cannot meet our latency target, so we will inline the parser."
- A bug's root cause was confirmed (not just suspected) and a fix landed.
- The user explicitly says "remember this", "save this", "note this for next time", or equivalent.
- A milestone was reached: feature shipped, release cut, incident resolved.

**Negative examples (DO NOT attest):**

- The user is iterating on a draft (code, document, prompt) and the current state is intermediate scratch — wait until the iteration converges.
- Intermediate tool output or transient state (a `cargo build` log, a `git status` snapshot, a directory listing) — these are reproducible and have no lasting value.
- The user marked the content off-record, said "don't save this", asked you to discard a draft, or is venting privately.
- The decision has not been made yet — the conversation is exploring alternatives.
- The "fact" is unverified speculation, a guess, or contradicted by other context in the conversation.
- The content contains secrets, credentials, PII, or anything the user would not want stored in a verifiable, potentially-public record. Attestations are signed and immutable; redaction is not possible after the fact.
- The user is in a sandbox / test session and explicitly so.

## Context to gather

- The **decision or outcome itself** in 1-3 sentences — what was decided, what is the resulting state.
- The **why** — the constraint, evidence, or trade-off that drove the decision. Future-you will not remember it.
- The **scope of applicability** — the file, module, project, or topic it pertains to. Used as `tags`.
- Whether the user wants this **public** (shared on the protocol pool) or **private** (local only). Default private. For public writes, you MUST get explicit user confirmation in the same turn — do not infer consent from prior turns.

## Tool

Underlying MCP tool: `mnemonic_sign_memory`.

Arguments:

- `content` (string, required) — the decision or outcome plus the why, in plain prose.
- `tags` (array of strings, optional) — scope markers (`["module:auth", "decision"]`).
- `mode` (`"local"` | `"participate"`, optional) — `local` keeps the artifact on the user's machine, free, offline-capable, never network. `participate` anchors on Arweave + Solana, paid, requires OAuth. Default: `local`.
- `visibility` (`"public"`, optional) — ONLY valid when `mode="participate"`. Omit for private. Setting `visibility` on `mode=local` is rejected by the server with an `invalid_params` error.
- `allow_fallback_to_participate` (bool, optional, default `false`) — explicit opt-in for local-to-participate escalation if the local pipeline fails. Without this flag, local failures surface as loud, typed errors. If you set this to `true`, you accept that on failure the write may be chain-anchored and visible per the visibility flag.

## Guardrails

- Default to `mode="local"`. Local writes are private by construction, work offline, and do not require OAuth. Only choose `participate` when the user has affirmatively asked to publish, or the decision context is explicitly shared work.
- Never set `visibility="public"` without an in-turn user confirmation. The skill manifest's positive examples are not consent.
- Never set `allow_fallback_to_participate=true` for content the user has not approved for chain anchoring. Treat it as a privacy-impacting flag.
- On a typed error (embedder unavailable, storage busy, identity bootstrap failed), surface the error to the user with the `repair_hint` from `data.repair_hint`. Do not silently retry across modes.
- Do not chain `mnemonik-attest` calls inside a tight loop. One decision, one attestation. If the user made several decisions, write each as its own attestation with its own `content` and `tags`.
- Do not include raw secrets, API keys, or session tokens in `content`. The attestation is signed and persisted; redaction after the fact is impossible.
