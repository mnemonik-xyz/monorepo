# Decisions — refresh-token-rotation

Append-only log of decisions and audit findings.

## Origin

This feature is the result of three scope-pivots during user-spec planning on
2026-06-06. Originally opened as `local-mode-survives-token-expiry`, then
narrowed to `session-reauth-recovery`, then renamed to
`refresh-token-rotation` after PKCE-constraint discovery and Stripe-MCP
precedent research.

## 2026-06-06 — Pivot 1: drop whitelist local+private

**Trigger:** User asked "Why we need any auth for local mode at all?"

**Finding (code-research §A):** On HTTP transport, `mode:"local"` writes
without JWT fall back to `owner_pubkey = server keypair` — producing rows
neither anonymous recall nor authenticated recall can return. Dead writes.

**Decision:** Drop AC1 (whitelist local+private). It silently makes the bug
worse, not better. Mode model cleanup reframed as separate feature
`work/binary-mode-cleanup/` (originally
`work/hosted-mode-rename-and-pricing/`, renamed after a further pivot
to binary-mode model later 2026-06-06).

## 2026-06-06 — Pivot 2: drop request_reauth

**Trigger:** Adequacy validator critical finding:
PKCE-binding makes server-side authorize URL synthesis impossible for the
Solana-wallet OAuth flow. PKCE state lives in the MCP host's process; the
server has no access to `code_verifier` or `code_challenge`.

**User challenge:** "User should sign, not server" — pointed out that JWT
represents server vouching, not user cryptographic identity. Deep
architectural point parked at `work/stateless-auth-rearch/`.

## 2026-06-06 — Pivot 3: refresh-token rotation (Stripe precedent)

**Trigger:** Researched how Stripe MCP handles auth. Findings:
- Local stdio mode: long-lived Restricted API Keys.
- Remote HTTP mode (`https://mcp.stripe.com`): OAuth 2.1 with 1h access +
  1y rolling refresh.

**Insight:** Stripe MCP works in Claude Desktop / Cursor / VS Code because
clients silently rotate refresh tokens. Standard OAuth 2.1.

**Insight on whitepaper invariant:** Cryptographic identity for memory bytes
is ALREADY user-owned (Ed25519 keypair signs COSE_Sign1 in browser-mediated
signing). JWT is session-routing only. Refresh tokens do NOT violate "user
owns identity".

**Decision A:** Implement standard OAuth 2.1 refresh-token rotation in
`/oauth/token`. Stripe-precedent: 1h access + 1y rolling refresh.

**Decision B:** Per-request signing parked at `work/stateless-auth-rearch/`
as long-term direction.

**Decision C:** Renamed `work/session-reauth-recovery/` →
`work/refresh-token-rotation/`.

## 2026-06-06 — Validation round 1 — applied fixes

Validators (completeness, quality, adequacy) returned 20 findings (3
critical, 8 major, 9 minor) on first pass of the rewritten spec.

**Critical findings & responses:**

- **C-completeness/quality (concurrent rotation atomicity)**: ROLL UP — see
  D11 + D13.
- **C-adequacy (R1 pre-ship empirical verification)**: ACCEPTED — R1
  mitigation rewritten to require one of two cheap pre-ship verifications
  (Option A: HTTP trace Claude Desktop ↔ Stripe MCP; Option B: dev deploy
  with JWT_TTL=60s). Pre-ship gate, not "smoke after ship".
- **C-adequacy (concurrent refresh + AC4 catastrophic)**: ACCEPTED —
  introduced D13 reuse-interval pattern (Auth0/Okta standard). Within 30s
  of rotation, the same old refresh returns the existing descendant pair
  idempotently. Only outside the window does presentation trigger
  family-revoke. Closes the kill-the-session race.

**Major findings rolled into decisions/spec:**

- M-completeness/M1 (dual content-type) → D14 + AC10.
- M-completeness/M2 (OAuthState DB handle) → D10.
- M-completeness/M3 (CLI cache out of scope) → D15 + Ограничения.
- M-completeness/m4 + M-quality (concurrent rotation) → D11 + D13.
- M-adequacy (1y TTL justification) → D2 expanded with rationale.
- M-adequacy (back-compat with old clients) → AC11.
- M-adequacy (hybrid 4h+refresh alternative) → considered, rejected;
  reuse-interval (D13) already solves concurrency, so trimming access
  TTL gives diminishing return. 1h kept.
- M-adequacy (AC4 UX position) → resolved by D13 reuse-interval —
  Auth0-pattern is the explicit position chosen.

**Minor findings rolled in:**

- m-completeness/m5 (eviction tick) → D12.
- m-completeness/m6 (TestServer mount) → Тестирование section + D-no-num
  spec text.
- m-quality (AC6 verification step) → Как проверить шаг 5 now explicitly
  checks `expires_at > expires_at_before`.
- m-quality (oauth/mod.rs:58 citation in Зачем) → reworded to "JWT access
  lives 1 hour" without file:line.
- m-quality (2 missing error scenarios — invalid_request + DB write fail)
  → AC13 + R6.
- m-adequacy (sha256 vs blake3 contradiction) → D1 fixed to blake3 with
  payment.rs:737-744 precedent.
- m-adequacy (no observability ask) → Ограничения now explicit:
  V1 = tracing logs only; Prometheus follow-up.
- m-adequacy (D7 forfeits "log out everywhere") → R7 + D16 explicit.

## Spec-time technical decisions (consolidated)

- **D1 (Refresh-token format)** — opaque 32-byte random, base64url for
  transport, blake3(salt+plaintext) stored. Plaintext returned once.
- **D2 (TTL)** — access 1h (unchanged), refresh 1y rolling. Stripe
  precedent + memory-protocol risk profile justification.
- **D3 (Rotation discipline)** — OAuth 2.1 §6.1 rolling rotation.
- **D4 (Family revoke on replay)** — only OUTSIDE reuse-interval (D13).
- **D5 (Storage location)** — refresh_tokens table in mcp/ (per CLAUDE.md).
- **D6 (No /oauth/revoke v1)** — no UX scenario requires it.
- **D7 (Access-token format unchanged)** — JWT HS256 1h. R7+D16 acknowledge
  the "no global logout" trade-off.
- **D8 (Discovery metadata)** — add "refresh_token" to grant_types_supported.
- **D9 (No Token Binding)** — HTTPS-only.
- **D10 (OAuthState gets DB handle)** — Arc<Mutex<Connection>> on shared
  store. Single DB file. Migration in mcp/src/oauth/refresh.rs by escrow.rs
  pattern.
- **D11 (Atomic rotation)** — single BEGIN IMMEDIATE transaction:
  SELECT-lock → check → UPDATE old → INSERT new. Pattern from payment.rs.
- **D12 (Eviction)** — hourly background sweep + opportunistic
  in-transaction cleanup of expired family siblings on each rotation.
- **D13 (Reuse-interval — Auth0/Okta pattern)** — 30s window after
  rotation; same old refresh within window returns existing descendant
  pair idempotently. Only outside window triggers family-revoke. Solves
  concurrent-401 race.
- **D14 (Dual content-type parity)** — refresh-grant parses both form-
  encoded and JSON via existing token_handler dispatch.
- **D15 (CLI cache out of scope)** — `~/.mnemonic/token.json` not extended
  in V1. Open `work/cli-refresh-token-support/` follow-up if requested.
- **D16 (No global logout v1)** — accept JWT access trade-off; document.
- **D17 (Not doing per-request signing now)** — deferred to
  `work/stateless-auth-rearch/`.

## Code-research dossier

See `code-research.md`:
- §A-§G: prior-pivot research (request_reauth allowlist, request_public_write
  template, recall behaviour, token model). Some sections informational
  rather than load-bearing now.
- §H: refresh-token-specific touchpoints — token_handler shape,
  storage placement (mcp/src/oauth/refresh.rs by escrow precedent), blake3
  hashing precedent, discovery metadata, eviction precedent, Claims schema,
  testing harness gaps.

## Renamed history

- `work/local-mode-survives-token-expiry/` → `work/session-reauth-recovery/`
  → `work/refresh-token-rotation/` (current).
- `work/binary-mode-cleanup/` (sibling parking lot, originally
  `work/hosted-mode-rename-and-pricing/`, renamed 2026-06-06 after
  binary-mode pivot).
- `work/chrome-extension-client-side-storage/` (sibling parking lot,
  opened 2026-06-06 as part of binary-mode pivot).
- `work/stateless-auth-rearch/` (long-term architectural option, opened
  2026-06-06).
