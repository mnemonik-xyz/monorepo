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
| 10 | **Bundler misbehavior** — the bundler (ArDrive Turbo) reorders or drops data items within a bundle | The anchored manifest root is the order-preserving root over `seq`-ordered hashes; the verifier re-derives it, so reordering/dropping is detected as #1/#5 | Integrity (detect) |
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

## Sealed memories boundary

Scope: the `SEALED_V1` artifact type and the write / recall / share / grant /
link flows. Code: `core/src/sealed/`, `mcp/src/sealed_routes.rs`,
`mcp/src/tools.rs` (sealed paths). Threat model tech-spec: §11 and §12 of
`work/sealed-memories/tech-spec.md`.

| # | Boundary | What enforces it | Class |
|---|---|---|---|
| S1 | **Plaintext never leaves client on private write** — on client-side paths (local MCP, CLI, extension, webapp) the inner `MEMORY_V1` CBOR is encrypted before any network call | `core/src/sealed/api.rs::seal_memory` runs in the client process; the tool argument or SDK call only transmits the `SEALED_V1` outer ciphertext | Integrity |
| S2 | **K zeroized after sealing** — the 32-byte content key must not persist after the wrap is made | `seal_memory` wraps `K` then zeroizes it via `Zeroizing<[u8;32]>`; `K` is never written to SQLite or Arweave | Integrity |
| S3 | **Server never receives K on share / link / E2E paths** — HPKE wrap sends `enc` + wrapped `K`; the server stores the grant but cannot open it | The grant endpoint stores `GRANT_V1` CBOR verbatim; `open_grant` runs client-side; `link_fragment` puts `K` in the URL fragment which browsers do not send in HTTP requests (RFC 3986 §3.5) | Integrity |
| S4 | **Approve page decrypts in browser before signing** — on the browser deferred-sign path the approval page decrypts the inner artifact to show the user what they are signing | `webapp/src/pages/Approve.tsx` calls `open_memory` (WASM) before rendering the approve dialog | Integrity |
| S5 | **kc verified before AEAD** — key commitment check happens before decryption; a key substitution attack fails immediately | `open_memory` and `open_with_key` verify `kc == blake3::derive_key("mnemonic sealed v1 key commitment", K)` and return `SealError::KeyCommitmentMismatch` on failure | Integrity |
| S6 | **Fragment never sent in HTTP requests** — the bearer-link puts `K` after `#`; browsers strip the fragment before sending | RFC 3986 §3.5 defines fragment as client-side only; `link_fragment` / `parse_link_fragment` encode / decode `K` in base64url after `#` | Integrity |
| S7 | **No third-party scripts on /m/ route** — the memory view page must not load external scripts that could read the fragment | CSP header on `/m/*` routes blocks third-party scripts; the page uses only WASM from the same origin | Integrity |
| S8 | **RK stored in RAM only, not SQLite** — the recall key (X25519 secret) derived from the identity key is held in an in-process cache and not written to the database | `core/src/identity/recall_key.rs` and `mcp/src/identity/unlock_cache.rs` hold the key in a `Mutex<Option<Zeroizing<[u8;32]>>>` and evict it on session end | Integrity |
| S9 | **Audit log: session start / end, no content** — the hosted recall session writes an audit record that does not include the plaintext | `mcp/src/sealed_routes.rs` writes `recall_session_start` / `recall_session_end` events; content is never included | Integrity |
| S10 | **PendingBundles holds no plaintext for sealed entries** — the deferred-sign bundle for a sealed write contains the outer ciphertext, not the inner plaintext | `mcp/src/pending.rs` stores the canonical CBOR bundle verbatim; the bundle is the SEALED_V1 outer artifact for a sealed write | Integrity |
| S11 | **sealed_blob never appears in vector search** — sealed rows are excluded from the cross-owner public vector search; their embeddings are not in the public index | `mcp/src/tools.rs::recall` skips rows where `sealed_blob IS NOT NULL` in the anonymous (public) query path; `sealed_hidden` is reported instead | Integrity |
| S12 | **Nonces from OS random source** — the 32-byte `K` and the 24-byte content nonce come from `OsRng` | `core/src/sealed/content.rs` uses `rand::rngs::OsRng` for both; no seed, no counter | Integrity |
| S13 | **All-zero shared secret rejected** — DHKEM with X25519 must reject the all-zero output per RFC 9180 §7.1.4 | The `hpke` crate enforces this; the wrap / unwrap path returns an error on the degenerate case | Integrity |
| S14 | **Small-order keys rejected** — X25519 public keys from small-order Ed25519 points are rejected | `core/src/sealed/wrap.rs` maps the Ed25519 verifying key to Montgomery form and checks for the small-order identity point before HPKE operations | Integrity |
| S15 | **Revocation not possible after grant delivery** — once a reader holds `K` they can keep a copy; the server can stop serving the grant record but cannot invalidate a copy | Stated explicitly in whitepaper §8.2, YELLOWPAPER §8.2, and in the `mnemonic_share` tool output before the grant is signed. There is no technical revocation mechanism. Authors must make a new version with a new `K` to stop future access. | Out-of-scope (design limit, documented) |
| S16 | **Hosted recall session: transient server-side exposure, opt-in only** — when the user opts in to a hosted recall session the server decrypts `SEALED_V1` artifacts in RAM to answer recall queries; the plaintext is not written to disk | `mcp/src/sealed_routes.rs` holds the decrypted text in a short-lived in-memory struct, writes nothing to SQLite, and zeroizes at session end. The user must explicitly start the session. The UI and tool docs state the exposure before the user opts in. | Availability / privacy (documented) |
