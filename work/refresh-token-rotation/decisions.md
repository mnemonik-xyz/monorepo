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
  Inside reuse-interval (5s) reused old token is treated as legitimate
  network-retry and returns the same descendant pair. Outside, it's
  treated as potential compromise and revokes the whole family by
  `family_id`.
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
- **D12 (Eviction)** — hourly background sweep only. Opportunistic
  in-transaction cleanup dropped in round 2 (overengineering for 1y TTL).
- **D13 (Reuse-interval — Auth0 pattern, 5s)** — 5-second window after
  rotation (Auth0 default; Okta default is 30s, deliberately not picked
  because longer window widens potential replay-attack timing). Same
  old refresh within window returns existing descendant pair
  idempotently. Only outside window triggers family-revoke. Solves
  network-retry race without compromising replay-detect.
- **D13.1 (`family_id` semantics)** — per-grant UUID. Each
  `authorization_code` exchange mints a fresh `family_id`. Multi-device
  users have multiple independent families; compromise of one doesn't
  revoke the others. Stripe/Auth0 standard. Alternative (sub-bound) was
  rejected as paranoid — single leak would kill all sessions.
- **D14 (Dual content-type parity)** — refresh-grant parses both
  `application/x-www-form-urlencoded` (today: VS Code, Claude.ai) and
  `application/json` (today: Cursor) via existing token_handler dispatch
  at `oauth/mod.rs:982-1078`. Round 2 corrected the inverted client
  attribution that was present in the round-1 draft.
  **Implementation hook (AC13)**: current `TokenRequest` struct
  (`oauth/mod.rs:946-957`) has no `grant_type` or `refresh_token`
  fields. Tech-spec needs to widen the struct AND add post-parse
  validation: if `grant_type=refresh_token` but `refresh_token` field
  is absent/empty → return `400 invalid_request`. The branch decision
  inside `token_handler` keys off `grant_type` after parse, not before
  (no new content-type dispatch).
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

## 2026-06-06 — Validation round 2 — applied fixes

Validators ran again on round-1-fixed draft, this round with targeted
focus on user modes / client surfaces / where content is stored (per
user's explicit ask). 13 findings (1 critical, 8 major, 4 minor).

**Critical & majors closed:**

- **Content-type inversion**: spec said "form for Cursor/VS Code, JSON
  for Claude Desktop"; code (`oauth/mod.rs:975-977`) is the opposite —
  VS Code + Claude.ai send form, Cursor sends JSON. PK
  `architecture.md` also has the bug; spec inherited it. Plus
  "Claude Desktop" was the wrong name throughout (Claude.ai is the
  correct product). Renamed and re-attributed in user-spec.md + D14.
- **Claude Desktop CORS evidence absent**: CORS allowlist covers
  `*.claude.ai`, not Claude Desktop's Electron origin. User's bug
  report from 2026-06-06 most plausibly originated in Claude.ai web
  (potentially loaded via desktop wrapper that shares the
  `https://claude.ai` origin). Target users renamed to "Claude.ai
  (web or desktop wrapper)" throughout.
- **`family_id` semantics undefined**: load-bearing for AC4. Picked
  per-grant UUID (D13.1) — multi-device isolation, Stripe/Auth0 norm.
- **AC4/D4 retry-vs-replay distinction implicit**: spec now explicit
  that reuse-interval IS the mechanism distinguishing legitimate
  retry-after-network-fail from replay-attack.
- **AC12 BEGIN IMMEDIATE serializes — `tokio::join!` test degenerate**:
  rewrote AC12 honestly — test exercises sequential-after-lock +
  reuse-interval lookup, which is correctness boundary that matters.
- **D13 30s mis-attributed**: 30s is Okta default, not Auth0. Auth0
  default is closer to 5s. Switched to 5s (D13) — tighter security
  window, sufficient for network retry.
- **AC10 verification gap**: added step 5b (JSON refresh-grant) to
  Агент проверяет table.

**Sibling drift corrections:**

- `binary-mode-cleanup/README.md`: Arweave description was
  "compressed embedding"; actually COSE_Sign1 envelope wrapping
  canonical CBOR. Reference to `patterns.md::Storage modes` was wrong
  section name; corrected. Updated separately.
- `chrome-extension-client-side-storage/README.md`: claim of
  in-extension fastembed via WASM was incorrect — `core/src/lib.rs:
  10-13` gates `embed` behind `#[cfg(not(target_arch = "wasm32"))]`.
  Architecture (`architecture.md:14`) envisions `ort-web` or
  `transformers.js` for browser. Updated the stub to be honest: WASM
  embedder is a sub-task of the migration, not free.
- `stateless-auth-rearch/README.md`: Option A wording ambiguous about
  whether stdio-proxy is side-grade or replacement; clarified as
  "full replacement of hosted OAuth path".

**Minors closed:**

- D12 dropped opportunistic in-transaction cleanup (overengineering).
- R1 Option A (TLS interception of Claude.ai ↔ Stripe MCP) demoted —
  needs custom CA in Electron trust store + possible cert pinning,
  high infra cost. Promoted Option B (`JWT_TTL=60s` dev deploy) as
  primary. Added Option C (read MCP SDK source).
- 13 ACs > M threshold (size_check minor): kept as-is; all tightly
  scoped to one endpoint. Acknowledged in interview.yml.
