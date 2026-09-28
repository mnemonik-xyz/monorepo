# Threat Model

Per-boundary threat models for Mnemonic. Each entry: attack → the check that
defeats it, and whether it is an **integrity** failure (cryptographically
defeated) or an **availability** failure (only *detectable*, not preventable).

---

## Verifiable Trajectories boundary

Scope: the `trajectory-experimental` surface — `STEP_V1` / `VERDICT_V1` /
`TRAJECTORY_V1` artifacts, `trajectory::verify_chain` / `verdict_coverage`, the
order-preserving Merkle `batch_root`, and the Arweave-bundle storage path. The
verifier is pure and backend-agnostic, so every check below runs client-side
with no trust in the server.

| # | Attack | Defeating check | Class |
|---|---|---|---|
| 1 | **Step reordering** — replay steps in a different order to fake a cleaner reasoning path | `verify_chain` requires `prev_hash[i] == content_hash[i-1]`, and `prev_hash` is inside the signed CBOR payload; the order-preserving `trajectory_root` also changes under reorder | Integrity |
| 2 | **Content tampering** — edit a step after the fact | `verify_chain` recomputes `blake3(canonical_cbor)` and compares to `content_hash`; the COSE_Sign1 signature over the payload fails | Integrity |
| 3 | **Verdict forgery** — fabricate a passing verdict | `verdict_coverage` verifies each verdict's COSE signature and that `signer == judge`; an unsigned/altered verdict does not count | Integrity |
| 4 | **Judge substitution / self-judging** — the producing agent signs its own "pass" | Coverage ignores any verdict where `judge == step.producer`; only independent judges count | Integrity |
| 5 | **Step omission / dense-range break** — drop an inconvenient step | `verify_chain` requires a dense `seq` range `0..n`; a gap sets `chain_valid=false` with `broken_at` at the gap | Integrity |
| 6 | **Batch-root mismatch** — anchor a root that doesn't match the steps | The root is recomputed from the steps' content hashes; inclusion proofs verify against the recomputed root, not the claimed one | Integrity |
| 7 | **Cross-trajectory replay** — reuse a step/verdict from another trajectory | Step `trajectory_id` and `seq` are in the signed payload; a step signed for trajectory A does not satisfy B's chain | Integrity |
| 8 | **Broken checkpoint root-of-roots** — splice two unrelated checkpoints | `verify_checkpoint_chain` requires each `prev_root == previous batch_root`; a spliced checkpoint breaks the chain | Integrity |
| 9 | **Settle-gate bypass** — act on a trajectory with a `reject` or missing coverage | `safe_to_settle = chain_valid && full coverage && no reject`; a single independent valid `reject`, or any uncovered step, forces `false` | Integrity |
| 10 | **Bundler misbehavior** — Irys/Bundlr reorders or drops data items within a bundle | The anchored manifest root is the order-preserving root over `seq`-ordered hashes; the verifier re-derives it, so reordering/dropping is detected as #1/#5 | Integrity (detect) |
| 11 | **Recall withholding** — a gateway hides steps to make a trajectory look complete or to hide a `reject` | Hiding a step breaks the dense range (#5); hiding a `reject` cannot create a *false positive* settle, but a withholding gateway *can* deny service. Mitigation: query multiple Arweave gateways / pin independently | Availability |
| 12 | **Anchor censorship** — the anchor chain refuses the root | The root is small and chain-pluggable (Solana SPL Memo / OpenTimestamps→Bitcoin); a censoring chain is swappable. Until anchored, the trajectory is unverifiable-in-time (detectable, not forgeable) | Availability |

### Residual / out-of-scope

- **Correctness of the model computation** (layer C): Mnemonic binds an external
  proof (`VERDICT_V1.proof_ref` + `proof_kind`) by hash but does **not** verify
  zkML/opML/TEE math. A verdict attests that *a judge* passed the step, not that
  the underlying inference was faithful. Trusting the verdict = trusting the
  judge identity.
- **Judge collusion**: an independent judge that is honest-but-bribed can sign a
  `pass` on a bad step. Defeated only by judge diversity / reputation, which is
  an ERC-8004 Reputation-Registry concern, not this layer.
- **Key compromise**: a stolen producer or judge key forges valid signatures.
  Out of scope here; covered by the identity/keychain boundary.

---

## Free anchor quota and replay boundary

Available now. Code: `mcp/src/payment.rs` (free quota, x402 nonces),
`mcp/src/client_ip.rs`, `mcp/src/api.rs` (`sign_callback_handler`). Audit log:
`work/free-quota-hardening/decisions.md`.

| # | Attack | Defeating check | Class |
|---|---|---|---|
| 1 | **Key farming** — mint many Ed25519 keys to get many free quotas | Free anchors need a Google-linked key (`google_identity_links`, possession proof at link time). The per-account counter is keyed on the Google account, so all keys of one account share one quota | Availability (cost) |
| 2 | **Account farming from one host** — many Google accounts from one machine | Per-IP share (`MNEMONIC_FREE_ANCHORS_PER_IP_PER_DAY`), IPv6 grouped by /64; global cap bounds the total daily spend | Availability (cost) |
| 3 | **Header spoofing** — send `X-Forwarded-For` to pick a rate-limit bucket or per-IP counter | Forwarding headers are read only when the TCP peer is in `TRUSTED_PROXIES`; the rightmost untrusted hop is used, never the leftmost | Availability |
| 4 | **Shared proxy bucket** — behind Caddy every user has the proxy's IP | The `tower_governor` limiters, the chat limiter and the per-IP counter use the resolved client IP (`ClientIpKeyExtractor`) | Availability |
| 5 | **Large free writes** — anchor big payloads at the operator's cost | COSE bytes above `MNEMONIC_FREE_ANCHOR_MAX_BYTES` take the paid path; content above `MNEMONIC_MAX_CONTENT_BYTES` is refused | Availability (cost) |
| 6 | **Refund farming** — force delivery failures to get the quota back and repeat | After the chain write starts, a refund gives back the account and IP counters but not the global one; the delivery-failure quota (`RefundsBySubject`) also applies | Availability (cost) |
| 7 | **Callback replay** — post one signed bundle twice or concurrently | The pending bundle is consumed once (`consume_by_id`); losers get 410 and their free anchor claim is refunded before any chain write; an anchored `content_hash` is never anchored again | Integrity (single anchor) |
| 8 | **x402 payment reuse** — one USDC transfer for two writes | `claim_x402_nonce` reserves the transaction atomically before the paid call; a failed call releases it; EVM hashes are normalized to lowercase | Integrity (single charge) |

### Residual

- A Solana or EVM transfer to the treasury is not bound to the payer: whoever
  presents an unused transaction first can use it. Binding needs a memo or
  reference in the transfer (planned, not available now).
- A paid delivery attempt whose lease expires (10 minutes) while its first run
  is still in progress can upload to Arweave or submit a memo a second time.
  The payment is not charged twice; the operator pays the extra chain fee.
- IP-based limits do not stop an attacker with many real IPv4 addresses. The
  global cap bounds the cost.

---

## A2A boundary

Scope: the A2A bridge surface — `bridge-a2a` sidecar, `mnemonic-a2a` library, MCP
`mnemonic_attest_a2a` tool, `bridge-a2a/fuzz/` fuzz target.  Objects in scope:
`Task`, `Message`, `Artifact` (all in `core/src/codec/a2a/`).  Signing and
canonicalization contract: RFC 8785 JCS → CBOR envelope (JCS bytes verbatim) →
COSE_Sign1 (Decision 1, `work/a2a-bridge/decisions.md`).

STRIDE rows:

| # | Category | Attack | Mitigation | Class |
|---|---|---|---|---|
| A1 | **Spoofing** | Identity substitution — AgentCard claims pubkey A but the COSE_Sign1 envelope is signed by keypair B (a different agent). A downstream consumer trusts the card's declared identity without checking the envelope signer. | Task 4's JWS-coverage check (`verify_a2a_attestation` with `expected_pubkey` set). The verifier compares `result.signer` against the card's declared key; a mismatch sets `valid = false`. | Integrity |
| A2 | **Tampering** | In-flight Task / Message mutation — an intercepting proxy edits a field after the COSE_Sign1 envelope is computed. | JCS canonicalization (`to_jcs_bytes`) produces a stable byte sequence that is signed verbatim. Any bit flip in the payload fails `verify_artifact` (COSE signature check). | Integrity |
| A3 | **Repudiation** | "I didn't send this message" — a producer later denies authorship of an attested message. | Per-message attestation stores the COSE_Sign1 envelope and the `signer_pubkey` in the SQLite row. The producer's keypair signed the exact JCS bytes; the attestation is a non-repudiation record. | Integrity |
| A4 | **Information disclosure** | Cross-tenant leakage via `recall_by_context` — tenant A guesses tenant B's `contextId` and retrieves attestations from a different agent workflow. | `contextId` is opaque per-tenant; the store filters by the calling identity's namespace. **Caveat:** if `contextId` values are predictable (e.g., sequential integers or task names in clear text), a tenant with read access could enumerate them. Mitigated by using UUIDs or content-addressed IDs for production contexts. | Availability / privacy (partially mitigated — guessing is possible with predictable IDs) |
| A5 | **Denial of service** | Flood attestation requests — an adversary floods `mnemonic_attest_a2a` with high-rate requests to exhaust the store or CPU. | Existing rate-limit middleware and x402 payment gate on `mnemonic_attest_a2a`. Free-quota hardening (see "Free anchor quota" boundary above) applies. | Availability |
| A6 | **Elevation of privilege** | Crafted A2A object triggers core panic or OOM — a malicious caller sends a pathological JSON payload (deeply nested, oversized, or invalid UTF-8) that causes the JCS or COSE path to panic or allocate unbounded memory. | `bridge-a2a/fuzz/` cargo-fuzz target (`fuzz_attest_message`) runs the full codec signing path on arbitrary JSON. CI gates PRs touching `core/src/codec/` or `work/a2a-bridge/` with a 30-minute fuzz run. serde_json's default recursion limit (128) prevents stack overflow on deeply nested input. | Integrity / Availability |
| A7 | **Canonicalization mismatch** | Two valid canonicalizations of "the same" object produce different bytes — a consumer re-serializes the A2A JSON and applies JCS, producing different bytes than the original canonical form, causing a spurious verification failure. | Decision 1 (`work/a2a-bridge/decisions.md`): the JCS bytes are **preserved verbatim** through the pipeline; the bridge never re-canonicalizes them. The `test_emitter_deterministic` regression test asserts byte-identical output across two runs. An explicit cross-language regression test in `@mnemonik-xyz/conformance` verifies the same hex from both Rust and TypeScript JCS implementations. | Integrity |
| A8 | **ContextId forking** | Two divergent attestation chains for the same `contextId` — two agents simultaneously attest under the same `contextId` with different `prev_id` values, creating a fork in the lineage. | The `prev_id` / `lineage` mechanism in `bridge-a2a/src/lineage.rs` surfaces forks: `recall_by_context` returns events in insertion order, so a consumer that checks monotonic ordering detects two competing chains. **Forking is not prevented at write time** (doing so would require consensus across agents). Operators are advised to use per-agent `contextId` namespaces and merge explicitly if they need a canonical chain. | Availability (detectable, not preventable without consensus) |

### Residual / out-of-scope for the A2A boundary

- **Key compromise**: a stolen agent keypair can produce valid COSE envelopes for arbitrary objects. Mitigated by key rotation (outside A2A bridge scope) and ERC-8004 validator revocation.
- **Semantic correctness**: the bridge attests that *an agent with keypair K* produced *these JCS bytes*; it does not verify the truthfulness or quality of the content. That is the reliability-oracle pattern (`docs/usecases/reliability-oracle-for-orchestration.md`).
- **SSE chunk attestation**: per-chunk attestation of streaming A2A responses is out of scope for V1 (see `work/a2a-bridge/decisions.md`). Streams are attested at the terminal task/artifact boundary only.
