# Decisions — free-quota-hardening

Append-only log. Each entry: date, who, what, why, and file:line evidence.
Line numbers refer to branch `claude/free-quota-hardening` at the commit that
adds this file.

---

## 2026-09-27 — Owner decisions implemented (claude)

### D1. Google account required for free anchors

- Linkage is decided **server-side from the database**, not from the JWT.
  `oauth::google::google_sub_for_pubkey` (`mcp/src/oauth/google.rs`, after
  `lookup_link`) reads `google_identity_links WHERE pubkey_base58 = ?`.
  Reasons: the sign-callback has no JWT (only the COSE-verified
  `signer_pubkey`); a wallet-login JWT for a Google-linked key has no
  `google_sub` claim; the first-touch Google JWT has `sub = google:<id>`, not
  a key. A link row exists only after `/oauth/google/link` verified a
  possession proof for the key, so nobody can claim another person's account.
- When one key is linked to several Google accounts, the earliest link wins
  (`ORDER BY linked_at, google_sub`), so a key maps to one stable account.
- The per-account counter subject is `blake3("free-anchor/google:" + sub)`.
  Every key linked to one Google account shares it. (Today
  `google_identity_links.google_sub` is the primary key, so one account links
  one key; the counter is ready for more.)
- A deploy without Google OAuth never creates the link table: that reads as
  "not linked" (no free anchors), not as an error.
- Not linked → `free_anchors: {eligible: false, reason:
  "google_account_required", link_hint}`. No JWT → `reason:
  "authentication_required"`. Paid x402 anchoring needs no Google link.

### D2. Per-IP share

- `MNEMONIC_FREE_ANCHORS_PER_IP_PER_DAY` (default 20). One `BEGIN IMMEDIATE`
  transaction bumps account, IP and global counters with `WHERE n < limit`;
  any miss rolls back all three (`payment.rs` `try_consume_free_anchor`).
- IP subject = `blake3_keyed(salt, "free-anchor/ip:" + bucket)`. The 32-byte
  salt is created once in `free_anchor_secret` by the migration, so the table
  never holds raw IPs and a leaked table cannot be brute-forced over IPv4.
  IPv6 is bucketed by /64 (`client_ip::rate_limit_key`).
- The IP charged is the **agent's** (the `mnemonic_sign_memory` caller),
  recorded on the parked bundle (`PendingEntry::requester_ip`), not the
  browser that posts the signature. Fallback: the callback's client IP. An
  unknown IP (in-process callers only) shares one bucket, so it never bypasses
  the limit.

### D3. Size limit

- `MNEMONIC_FREE_ANCHOR_MAX_BYTES` default **16 KiB** of COSE_Sign1 bytes (the
  exact bytes uploaded to Arweave). Measured by
  `payment::tests::typical_artifact_size`: a 1026-byte memory with a 384-dim
  (fastembed, 4-bit) embedding makes a canonical CBOR of 1589 bytes and a
  COSE_Sign1 of 1729 bytes (about 1.7 KiB). A 1536-dim (OpenAI) embedding adds
  768 base64 bytes, about 2.5 KiB (computed, not measured: that compressor is
  too slow to build in a unit test). The COSE envelope adds 140 bytes over the
  canonical CBOR (`COSE_SIGN1_OVERHEAD_BYTES = 256` is the bound). 16 KiB
  leaves room for about 14 KiB of text while bounding a free write's fee.
- Checked twice: at parking (`canonical_cbor.len() + 256`; an oversized bundle
  is discarded and the call falls through to the 402 with `reason:
  "too_large"`), and exactly at the callback (`cose_bytes.len()`).
- Hard maximum audit: before this change the only limits were the 1 MiB body
  peek in `oauth::bearer_auth_middleware` (`MAX_PEEK_BODY`) and the 2 MiB
  body limit in `mcp_handler` (`to_bytes(.., 2 * 1024 * 1024)`), axum's default
  2 MB JSON limit on `/api/sign-callback`, and the 32 KiB `content` cap in
  `PendingBundles::insert` (deferred path only). The stdio / inline path had
  no content limit. Added `MNEMONIC_MAX_CONTENT_BYTES` (default and ceiling
  32 KiB, equal to the pending cap) checked in the `mnemonic_sign_memory`
  dispatcher for every transport and mode, before the embedder runs.

### D4. Real client IP for rate limits

- `tower_governor` used the default `PeerIpKeyExtractor` (TCP peer). Behind
  Caddy every user shared Caddy's bucket. `chat.rs` keyed on the peer too.
- New `mcp/src/client_ip.rs`: `TRUSTED_PROXIES` (CIDR list; default
  `127.0.0.0/8,::1/128,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7`,
  which covers the Docker `caddy` network in `docker-compose.yml`). Forwarding
  headers are read only when the peer is trusted; the rightmost untrusted
  `X-Forwarded-For` hop wins; an unparsable hop stops the walk; `X-Real-IP` is
  the fallback. A malformed list aborts the boot.
- One resolver feeds `ClientIpKeyExtractor` (all three governor configs in
  `main.rs`), the chat limiter, and the per-IP free counter.
- Tests: `client_ip::tests::*` (spoofing ignored from an untrusted peer,
  rightmost hop, IPv6 /64) and
  `free_anchor_quota::per_ip_share_counts_the_real_client_ip_and_ignores_spoofed_headers`.

### D5. No global refund after a chain write

- `FreeAnchorGrant::mark_chain_write` is called right before the Arweave
  upload starts (an upload can be billed even when its response is lost).
  After it, the drop-refund gives back the account and IP counters only
  (`refund_free_anchor(.., refund_global = false)`). Before it, all three.
- Test: `free_anchor_quota::delivery_demotion_refunds_the_free_anchor` now
  expects `(10, 999)`; unit test
  `refund_after_a_chain_write_keeps_the_global_counter_spent`.

---

## 2026-09-27 — Replay audit (claude)

### R1. `/api/sign-callback`, same correlation_id / same COSE bytes

- **Already safe:** the non-paid path consumes the in-memory bundle once
  (`pending.rs:350` `consume_by_id`, pop under the tokio mutex); a second post
  gets 410 (`api.rs:563`). Expiry: `peek_by_id` and `consume_by_id` reject
  `exp <= now` (`pending.rs:337`, `pending.rs:360`); TTL 300 s. Owner binding
  (`api.rs:247`) and COSE kid + hash checks (`api.rs:253` onward).
- **Concurrency note:** concurrent duplicates all pass the peek and may each
  claim a free anchor before one wins `consume_by_id`. Losers drop their grant
  before any chain write, so the refund is full. Net: one anchor, one counter
  unit. New test
  `concurrent_duplicate_callbacks_anchor_once_and_consume_one_free_anchor`
  (8 parallel posts) and `the_same_callback_posted_twice_anchors_once_...`.
- **Paid path already safe:** `stage_verified_cose` binds a correlation id to
  one exact envelope (`paid_artifact.rs:170`, conflict at `:204`);
  `acquire_delivery_attempt` refuses `completed`/`abandoned` or a live lease
  (`paid_artifact.rs:378`), so a completed bundle cannot be reused (409).
- **Note (not changed):** a staged paid context recovered from the database
  (`api.rs:219`) has no 300 s expiry on purpose: payment settlement may take
  longer. It can never get a free anchor and anchor for free: a free grant
  switches off the paid rail, and `consume_by_id` then finds no in-memory
  bundle and returns 410 (grant refunded).

### R2. Re-submitting an already-anchored artifact

- The server builds each artifact with a fresh `artifact_id` (UUID) and
  `created_at` (`tools.rs` `sign_memory_deferred`), so a new
  `mnemonic_sign_memory` call can never produce an existing `content_hash`;
  the same text twice is two memories and is counted twice (by design).
- **Fixed (defense in depth):** before anchoring, the callback looks up an
  existing `participate` row with the same `content_hash`
  (`SqliteStore::find_anchored_by_content_hash`, new index
  `idx_attestations_content_hash`; `api.rs:605`). If one exists it returns
  the existing anchor with `already_anchored: true`, writes nothing, charges
  nothing, and the free grant drops with a full refund. For a paid attempt it
  marks the delivery completed. Test:
  `an_already_anchored_content_hash_is_not_anchored_again`.
- **Fixed:** a retry of the same correlation id reuses the attestation row it
  wrote before (`find_by_correlation_id` → same `attestation_id`,
  `INSERT OR REPLACE`; `api.rs:650`). Before, every paid delivery retry after
  a demotion inserted another row for the same artifact.

### R3. x402 payment signature / nonce reuse

- **Fixed (real double-spend):** the nonce was inserted only after the tool
  call succeeded (`consume_x402_nonce_after_success`), and a unique-constraint
  failure was only logged. Two concurrent requests with one `X-Payment` both
  passed the read-only check (`payment.rs:560`), both verified on chain, both
  parked a paid (unflagged) bundle, and both could anchor: one payment, two
  anchors. Now `mcp_handler` reserves the payment atomically with
  `claim_x402_nonce` (`INSERT OR IGNORE` on the primary key,
  `payment.rs:631`; call site `mcp.rs:1649`) before the call, returns 401 to
  the loser, and releases it on any tool error (`release_x402_nonce`,
  `payment.rs:646`; `mcp.rs:1755`) so a failed delivery keeps the USDC
  reusable, as before. Tests: `x402_nonce_claim_is_single_use_until_released`,
  `concurrent_x402_claims_have_exactly_one_winner`, and the existing
  `delivery_guarantee` nonce test.
- **Fixed:** EVM tx hashes are case-insensitive at the RPC but the nonce
  table compared them case-sensitively: `0xABC…` and `0xabc…` were two
  nonces for one payment. `normalize_x402_proof` (`payment.rs:143`) stores
  lowercase `0x…`; Solana base58 is only trimmed. Test:
  `evm_tx_hash_spellings_normalize_to_one_nonce`.
- **Residual (documented, not fixed):** a transfer is not bound to the payer;
  whoever presents an unused transfer to the treasury first may use it. Needs
  a memo/reference binding (planned).

### R4. Universal Paywall operation / quote reuse

- **Already safe:** the operation is bound to the payer subject hash and the
  COSE artifact hash (`payment.rs:281`); a `PaymentReady` operation proceeds
  without a second settlement (`payment.rs:284`); the provider settles one
  EIP-3009 authorization (nonce enforced on chain); a quoted operation never
  takes a free anchor (`claim_free_anchor`, `PaidOperationInProgress`).
  Delivery after payment is guarded by the delivery-attempt lease (R6).

### R5. OAuth

- **Identity-login challenge — already safe:** the pending challenge is popped
  single-use under the mutex (`oauth/mod.rs:1294`), expiry checked
  (`oauth/mod.rs:1303`), and the Ed25519 signature is verified against the
  server-stored canonical-CBOR challenge bytes that contain the nonce, state
  and expiry.
- **Authorization code — already safe:** popped single-use
  (`oauth/mod.rs:1625`), expiry (`:1633`), PKCE S256 (`:1657`), redirect URI
  binding; test `oauth_flow` "code must be single-use even after a 400".
- **Refresh token — already safe:** rotation with a 5 s idempotent reuse
  window, reuse outside the window revokes the whole family in one
  transaction (`oauth/refresh.rs:1166` test
  `replay_outside_window_revokes_full_family_in_one_tx`;
  `tests/oauth_refresh_e2e.rs:352` `test_replay_outside_reuse_revokes_family`).
- **Google link challenge — already safe:** popped single-use
  (`oauth/google.rs:286`), 5-minute expiry (`oauth/google.rs:804`).

### R6. Paid-delivery retry (`resume_due_paid_deliveries`)

- **Already safe against double charge:** the worker has no payment proof; it
  re-enters `sign_callback_handler` with staged bytes (`api.rs:128`), and
  `acquire_delivery_attempt` resumes from the recorded `arweave_tx` /
  `solana_tx` instead of writing again (`api.rs:526`, `api.rs:732`,
  `api.rs:816`).
- **Fixed:** duplicate attestation rows on retry (R2).
- **Residual (documented, not fixed):** if an attempt runs longer than its
  10-minute lease, or `record_arweave_uploaded` / `record_solana_submitted`
  fails, a later attempt can upload or submit a memo a second time. No second
  charge; the operator pays the extra chain fee. A fix needs a two-phase
  "submitting" state; out of scope here.
