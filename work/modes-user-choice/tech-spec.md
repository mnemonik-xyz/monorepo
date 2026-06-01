---
created: 2026-06-01
status: draft
size: L
branch: dev
related:
  - work/modes-user-choice/user-spec.md
---

# Tech Spec: Mode as a per-request user choice, with whoami envelope contract

## Problem recap

`STORAGE_MODE` and `PAYMENT_MODE` are read once in `mcp/src/config.rs::from_env`
and threaded as `&str` into every tool (`whoami`, `sign_memory`, `recall`,
`verify`) and into the dispatch + paywall in `mcp/src/mcp.rs`. The operator
picks one value for the whole process; the caller cannot choose. CLAUDE.md
encodes the consequence as two rules: *"Mode is set at startup, not per-call"*
and *"Never mix in one DB"*. This spec deliberately revisits both.

The user-spec (canonical) reduces the surface to a single per-write intent:

- **`local`** — artifact stays on the user's own filesystem/self-hosted store.
  Free, offline, private. Protocol-guaranteed free path (whitepaper §5.7.1).
- **`participate`** — artifact is anchored on Arweave + Solana and proved
  retrievable. Paid service-layer path; "delivered = anchored AND verified by
  recall."

The shipped `work/chrome-extension/` (Local/Cloud tiers, Cloud = thin client to
hosted `STORAGE_MODE=full` via deferred signing) must keep working unchanged —
"`mode` is optional, default `local`; requests without it follow the legacy
env-var behavior" is a hard backward-compatibility invariant.

## Solution

Three coordinated changes.

1. **Per-request mode field.** `mnemonic_sign_memory` accepts an optional
   `mode: "local" | "participate"` (default `local`). The mode resolves once
   at request entry and drives the rest of the pipeline (paywall, anchor or
   not, cost recording, delivery proof). Requests without `mode` resolve to
   the legacy env-var (`storage_mode`) → backward-compatible.

2. **`whoami` envelope (discoverability contract).** `mnemonic_whoami`
   gains `supported_modes`, `default_mode`, `participate_cost` fields. A pure
   `STORAGE_MODE=local` server advertises `["local"]` only; a `full` server
   advertises both. A client requesting `participate` against a `local`-only
   server gets a typed JSON-RPC error (`code: -32010, kind: "UnsupportedMode"`)
   — **never** a silent downgrade.

3. **Delivery guarantee + storage tagging.** Every attestation row gets a
   `write_mode TEXT NOT NULL` column (`local` / `participate`). A `participate`
   write only succeeds after a **recall + verify round-trip** against the
   anchored bytes; on failure the row is downgraded to `local` and **no
   payment is charged**. `recall` spans both modes for the same owner;
   `verify` routes by the stored `write_mode` instead of the env-var.

Tier-2 ("run your own MCP") is not new code — it is existing
`STORAGE_MODE=full + PAYMENT_MODE=none`, now positioned explicitly and made
discoverable through the envelope (`participate_cost.amount_cents: 0,
payment_methods: []`). No new binary, no new path, only the existing one made
visible.

## Architecture

### What we touch

- `core/src/storage/mode.rs` — **new**, pure type, no I/O.
- `core/src/storage/sqlite.rs` — add `write_mode` column via idempotent
  migration helper (mirroring `migrate_owner_pubkey_columns`); backfill rule:
  `solana_tx LIKE 'local:%' → 'local'`, else `'participate'`. Extend
  `save_attestation` and `AttestationStore` trait with a `write_mode` parameter.
- `core/src/storage/traits.rs` — `AttestationStore::save_attestation` signature
  change (additional `WriteMode` parameter).
- `mcp/src/tools.rs`:
  - `sign_memory_inline` — parse `mode` from input, resolve against
    `WriteMode`, branch on resolved mode (today branches on the
    `storage_mode: &str` argument in two places — local-vs-Arweave write
    block and the cost-recording block); on `Participate`, run the
    recall+verify round-trip before persisting "delivered" state.
  - `whoami` — return envelope.
  - `verify` — route by stored `write_mode` column.
- `mcp/src/mcp.rs` — paywall gate inside `mcp_handler`. Today:
  `payment_mode != "none" && storage_mode != "local"`. New: gate on
  resolved `WriteMode::Participate`. Requests without `mode` field continue
  to fall back to env-var-driven legacy behavior (compat).
- `mcp/src/payment.rs` — no API change; only callsite reshuffles. `Local`
  writes never reach `check_payment` / `record_attestation_cost`.
  T3 wires `refund_balance` for the delivery-failure path.
- `CLAUDE.md` (root) — retire the "Never mix modes in one DB" rule
  explicitly in the same PR.

### Shared resources

None added. Existing `McpState` (one Arweave client, one Solana client, one
pricing engine, one SQLite pool) is unchanged in shape.

### Data flow (`participate` write, new)

1. Caller sends `tools/call mnemonic_sign_memory { mode: "participate", … }`.
2. Dispatcher resolves `WriteMode::Participate`. Paywall fires (existing
   `payment::check_payment`). Funds reserved, not yet charged.
3. `sign_memory_inline` runs the normal embed → compress → CBOR → blake3 →
   COSE pipeline.
4. Arweave write + Solana memo as today.
5. **New:** in-process recall against the just-written `content_hash` plus
   `verify_cose` over the *anchored* bytes (re-fetched from Arweave). Must
   both succeed.
6. On success: persist row with `write_mode='participate'`, call
   `record_attestation_cost`, return success envelope including
   `delivery_receipt { recall_verified_at, arweave_tx, solana_tx }`.
7. On failure (anchor 200 but not retrievable, or hash mismatch): persist
   row with `write_mode='local'` (so the embed/signature aren't wasted),
   refund the reserved payment, return error
   `kind: "DeliveryNotConfirmed"`, no charge.

### Data flow (`local` write, new)

Identical to today's `storage_mode=local` branch. Synthetic `local:` ids,
SQLite-only, `write_mode='local'`. Paywall is bypassed entirely (compile-time
property: the gate only fires when resolved mode is `Participate`).

## Decisions

1. **Per-request `mode` is the source of truth; env-var becomes fallback for
   legacy clients only.** [supports user-spec invariant "Гранулярность —
   per-request"]. Reason: silently honoring `storage_mode` against an explicit
   `mode` value would violate the user's intent.
2. **`mode` field is optional, defaults to `local`, falls back to env-var when
   absent.** [supports compatibility invariant for shipped chrome-extension].
   The shipped extension never sends `mode`; its Cloud-tier (deferred-signing
   → `STORAGE_MODE=full`) must keep working byte-for-byte. Test: existing
   `tests/sign_memory_deferred_*` suite must pass unchanged.
3. **`whoami` envelope contract** (`supported_modes`, `default_mode`,
   `participate_cost`). [supports user-spec invariant "Discoverability через
   whoami"]. Envelope derived from `Config` at process start (cached); no
   runtime probe.
4. **Typed JSON-RPC error `UnsupportedMode` (`code: -32010`) on a mode the
   server cannot serve.** [supports user-spec invariant "типизированная
   ошибка, не тихий downgrade"]. JSON-RPC reserves `-32000..-32099` for
   "server errors"; `-32010` is unused in our existing error space (see
   `mcp/src/mcp.rs` error helpers).
5. **Storage invariant S1: one DB, rows tagged by `write_mode`.** [supports
   user-spec invariant "сосуществуют в одной БД, тегируясь `write_mode`"].
   Backfill rule for legacy rows: `'local'` if `solana_tx LIKE 'local:%'`,
   else `'participate'`. CLAUDE.md "Never mix in one DB" rule is retired in
   the same PR.
6. **Delivery proof = recall+verify round-trip over anchored bytes.**
   [supports user-spec invariant "Доставлено = заякорено И подтверждено через
   recall"]. Uses existing `verify_cose` helper in `mcp/src/tools.rs` and
   the SQLite cosine recall path — no new primitive.
7. **On delivery failure: row demoted to `local`, no charge.** [supports
   user-spec "плата не берётся" + "запись остаётся local"]. Implementation:
   the participate path persists the row with `write_mode='local'` and skips
   `record_attestation_cost`; the reserved balance/x402 payment is released
   via `payment::refund_balance` (in `mcp/src/payment.rs`, currently invoked
   from `mcp/src/mcp.rs::mcp_handler` on tool-execution errors). Refund
   reason must include the demoted `attestation_id` for 1:1 correlation
   with the downgrade. A refund-itself failure writes a structured audit
   row (new in T3) instead of only `tracing::warn!` (current behavior).
8. **No `.await` across in-process lock guards (SQLite mutex AND DashMap
   shard) in the participate flow.** [supports project pattern "storage
   lock discipline" — see patterns.md "Storage lock discipline"; extended
   to DashMap by this spec because the new `RefundsBySubject` introduces a
   second same-class lock]. T3 inserts new awaits (Arweave re-fetch,
   `verify_cose`, recall) between the Solana memo and the final DB persist.
   The implementation uses two short critical sections for the SQLite
   mutex (post-anchor success persist OR post-failure demote+refund) and
   read-only/write-and-drop access patterns for DashMap shard guards;
   neither lock class is held across an `.await`. Code-audit (A1) checks
   both lock classes.
9. **`find_by_tx` must filter by `owner_pubkey` to preserve tenant isolation.**
   [supports patterns.md "Tenant isolation via `owner_pubkey`"]. With `local`
   and `participate` rows now coexisting for many tenants in one DB,
   `verify`'s routing lookup (T4) cannot fall through `find_by_tx` unscoped.
   T4 closes this pre-existing gap as part of the routing change.
10. **[TECHNICAL] Trait signature change is breaking inside the workspace.**
    `AttestationStore::save_attestation` gains a `WriteMode` parameter. All
    internal callsites in `core/src/storage/sqlite.rs` (test helpers) plus
    `mcp/src/tools.rs::sign_memory_inline` and
    `mcp/src/api.rs::sign_callback_handler` must be updated in the same
    task. No external crates depend on this trait.

## User-Spec Deviations

None. All tech-spec decisions trace to user-spec invariants (anchor IDs in
each Decision above).

## Data model

### Schema migration

Idempotent helper `migrate_write_mode_column` (mirrors
`migrate_owner_pubkey_columns` in `core/src/storage/sqlite.rs`):

```sql
-- 1. Add column with safe default for fresh schemas.
ALTER TABLE attestations
  ADD COLUMN write_mode TEXT NOT NULL DEFAULT 'participate';

-- 2. Backfill legacy rows based on tx-id shape.
--    `local:_%` requires at least one character after the colon; this is
--    strict-collision-safe with real Solana signatures because base58
--    excludes lowercase `l` and ':' is not in base58 at all.
UPDATE attestations
   SET write_mode = 'local'
 WHERE solana_tx LIKE 'local:_%';

-- 3. Index for filtered recall + audit queries.
CREATE INDEX IF NOT EXISTS idx_attestations_write_mode
  ON attestations(owner_pubkey, write_mode);
```

`DEFAULT 'participate'` is the conservative choice: a row that existed under
the legacy global `STORAGE_MODE=full` operator was, by definition, a paid
participate write. The `LIKE 'local:_%'` backfill catches the
`storage_mode=local` rows independently. The `local:` prefix is reserved as
the synthetic-id namespace owned exclusively by `tools.rs::sign_memory_inline`
(local branch) — documented in the same PR as a hard invariant.

### `WriteMode` enum

```rust
// core/src/storage/mode.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode { Local, Participate }
```

Round-trip serde: lowercase strings on the wire; rusqlite `ToSql`/`FromSql`
via `String` adapter.

## API contract changes

### `mnemonic_sign_memory` (input addition)

```jsonc
{
  "content": "...",
  "tags": ["..."],
  "mode": "local" | "participate"   // NEW, optional, default = legacy env-var
}
```

**Resolution rule (single source of truth, used by *both* the paywall gate and
the persisted `write_mode` column):**

| Input `mode` value | Resolves to | Notes |
|---|---|---|
| field absent (key missing) | env-var fallback (`local` if `STORAGE_MODE=local`, else `participate`) | Backward-compat for the shipped extension. |
| `"local"` (exact lowercase string) | `WriteMode::Local` | |
| `"participate"` (exact lowercase string) | `WriteMode::Participate` | |
| `null` / non-string / empty string / whitespace / different case / unknown string | JSON-RPC `-32602 InvalidParams` (`data.field: "mode"`, `data.received: <verbatim>`) | Reject early in the dispatcher — never reaches the gate or the storage column. |

The resolution function is pure and lives in `mcp/src/tools.rs` (T2);
`mcp.rs::mcp_handler` calls it once and threads the resulting `WriteMode`
into both the paywall check and `sign_memory`. Two-path drift is impossible
by construction.

### `mnemonic_whoami` (output addition)

```jsonc
{
  "public_key": "...",
  "did_sol": "...",
  "did_key": "...",
  "attestation_count": 42,
  "storage_mode": "full",                  // KEPT (legacy field, still env-var)
  "supported_modes": ["local","participate"],   // NEW
  "default_mode": "local",                       // NEW
  "participate_cost": {                          // NEW (null on local-only)
    "currency": "USD",
    "amount_cents": 1,
    "payment_methods": ["x402","balance"]
  }
}
```

### `mnemonic_sign_memory` success envelope (participate, new field)

```jsonc
{
  "...existing fields...",
  "write_mode": "participate",
  "delivery_receipt": {
    "arweave_tx": "...",
    "solana_tx": "...",
    "recall_verified_at": "2026-06-01T12:34:56Z"  // operator-attested, see note
  }
}
```

**Trust model for `recall_verified_at`.** This timestamp is **server-set** at
the moment the recall+verify round-trip passes. It is *not* part of the
COSE-signed payload and *not* chain-anchored. The cryptographically
verifiable timestamp is the Solana memo's `block_time`, which any third
party can fetch from `solana_tx`. `recall_verified_at` is an operator-level
receipt asserting "I successfully read these bytes back at time T"; it is
useful for SLA/SLO purposes but third-party verifiers should treat
`solana_tx.block_time` as the canonical anchor time. The user-spec
invariant ("anchored AND verified by recall") is satisfied because the
*successful return* of `sign_memory` is itself the operator's claim of
verified retrievability — the timestamp is metadata.

### Typed errors

```jsonc
// Unsupported mode (server cannot serve requested mode)
{ "code": -32010, "message": "Unsupported mode",
  "data": { "kind": "UnsupportedMode",
            "requested": "participate", "supported": ["local"] } }

// Delivery not confirmed (anchored but read-back failed)
{ "code": -32011, "message": "Delivery not confirmed",
  "data": { "kind": "DeliveryNotConfirmed",
            "arweave_tx": "...", "solana_tx": "...",
            "stage": "recall" | "verify", "row_demoted_to": "local" } }
```

## Testing strategy

Feature size **L** → unit + integration + targeted E2E.

### Unit
- `core/src/storage/mode.rs` — serde round-trip, rusqlite round-trip.
- `migrate_write_mode_column` — fresh-schema path, legacy-rows backfill path
  (mix of `local:abc`, `local:` exact, and real signatures), idempotency
  (run twice, same result).
- `parse_mode` request resolver — accepts `"local"`, `"participate"`; missing
  (→ env-var fallback); rejects `null`, non-string, `""`, `" "`, `"Local"`,
  `"PARTICIPATE"`, unknown — each producing `-32602 InvalidParams`. The
  resolver is the single source feeding the paywall gate and the persisted
  `write_mode` column (drift-impossible by construction).
- `paywall_gate(WriteMode)` — pure function returning whether to charge;
  `WriteMode::Local` always false. (Whoami envelope shape is covered at
  integration level; no unit duplicate.)

### Integration (Rust, `mcp/tests/`)
- `mcp/tests/modes_per_request.rs` (new) — drives the MCP HTTP dispatcher
  end-to-end:
  - `sign_memory { mode: "local" }` against a `STORAGE_MODE=full` server →
    free, no Arweave write, row tagged `local`.
  - `sign_memory { mode: "participate" }` against `STORAGE_MODE=local` server
    → `UnsupportedMode` (`-32010`), no row written.
  - `sign_memory { /* no mode */ }` legacy path — golden-fixture assertion
    against the response shape the shipped extension consumes; any field
    drift fails the test (regression guard).
  - `whoami` envelope shape across the three deploy variants: local-only,
    self-operator (`full + PAYMENT_MODE=none`), hosted-x402.
  - **Mixed-mode coexistence:** seed one DB with a `local` row and a
    `participate` row for the same `owner_pubkey`, call `recall`, assert
    both surface in the result and each carries its stored `write_mode`.
- `mcp/tests/delivery_guarantee.rs` (new) — uses the existing `arlocal` +
  `solana-test-validator` harness:
  - happy path: anchor → recall+verify succeeds → row `participate`,
    `delivery_receipt` returned, `attestation_costs` row written.
  - induced failure (`PAYMENT_MODE=balance`): single-server `httpmock`
    returning *corrupted* GET bytes on the Arweave re-fetch (pattern in
    `core/src/arweave/mod.rs::new_for_test`); row demoted to `local`, no
    `attestation_costs` row, JSON-RPC `-32011 DeliveryNotConfirmed`, **and**
    the caller's balance returns to pre-call value (refund-release assertion,
    not just absence of cost row).
  - induced failure (`PAYMENT_MODE=x402`): same demotion path, plus assert
    the x402 nonce is **not** marked consumed in `x402_nonces` — the caller
    can retry with the same payment. Closes the x402 corner the
    balance-only assertion leaves open.
  - DoS guard: simulate N repeated demotions for one owner → the
    (N+1)-th `participate` call returns `-32011 DeliveryQuotaExceeded`
    before any Arweave or Solana call (asserted by *absence* of mocked
    Arweave/Solana invocations in that call).
- `mcp/tests/verify_by_stored_mode.rs` (new) — explicit scenarios for T4:
  - row stored with `write_mode='local'` → `verify` routes through
    `verify_local` regardless of `STORAGE_MODE` env-var.
  - row stored with `write_mode='participate'` → `verify` routes through
    `verify_cose`.
  - **tenant isolation:** caller A's `verify` against caller B's
    `solana_tx` returns "not found", never leaks `content_hash`,
    `signer_pubkey`, or content preview (regression guard for the
    `find_by_tx` gap closed in T4).

### Compatibility regression
The shipped extension's Cloud-tier exercises the deferred-signing path
(`sign_memory_deferred`, `/api/sign-callback`). The existing tests
`mcp/tests/deferred_sign_flow.rs` and `mcp/tests/sign_callback.rs` must pass
unchanged at the *assertion* level — the new `mode` field is parsed once at
the top of `sign_memory` and the deferred branch (`jwt_sub.is_some()`) is
entered before any mode-specific code path, so its behavior is byte-identical
when `mode` is absent. T1's trait signature change will require updating
internal test-helper callsites inside those files, but no test assertion or
HTTP contract changes. A golden response-shape fixture (added in T2) pins
the mode-absent response envelope against future drift.

### Not covered by this feature
- Browser-side behavior — out of scope (kept compatible, not modified).
- Pluggable signer / chain-pluggable anchor (issue #29).
- Wallet-connect UX (browser-side, future).

## Implementation Tasks

### Wave 1 — Foundation (core/)

**T1: WriteMode type + DB schema migration + save_attestation signature**
- Description: Introduce `WriteMode { Local, Participate }` as a pure type
  in `core/`. Extend the `AttestationStore` trait and `SqliteStore` so every
  row persists its mode. Add an idempotent migration helper that adds the
  column and backfills legacy rows by the synthetic-id prefix rule defined
  in Data model §"Schema migration". Update all internal callsites
  (sqlite.rs tests, fixtures).
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`
- Verify-smoke: `cargo test -p mnemonic-core storage::` passes; run
  `cargo test -p mnemonic-core migrate_write_mode_` to exercise both
  fresh-schema and legacy-DB paths.
- Files to modify: `core/src/storage/mode.rs` (new), `core/src/storage/mod.rs`,
  `core/src/storage/sqlite.rs`, `core/src/storage/traits.rs`, `core/src/lib.rs`.
- Files to read: `core/src/storage/sqlite.rs` (migration helpers at
  `migrate_owner_pubkey_columns`, `migrate_correlation_id_column`;
  `save_attestation` impl), `core/src/storage/traits.rs` (`AttestationStore`
  trait), user-spec.md.

### Wave 2 — API surface (mcp/)

**T2: Per-request `mode` field + UnsupportedMode error + paywall reframing + whoami envelope + golden fixture**
- Description: Add an optional `mode` field to `mnemonic_sign_memory` and a
  single resolver function (rules in API contract §"Resolution rule").
  Reroute the paywall gate in `mcp/src/mcp.rs` to fire on resolved
  `WriteMode::Participate`. Extend `whoami` output with the envelope
  contract. Emit `-32010 UnsupportedMode` when the caller requests a mode
  outside `supported_modes`; reject malformed `mode` values with
  `-32602 InvalidParams`. Thread `WriteMode` into `sign_memory_inline` and
  `save_attestation`. Add a golden-fixture test asserting the mode-absent
  response shape (compat guard for shipped extension).
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`
- Verify-smoke: `cargo test -p mnemonic-mcp` passes; quick HTTP probe —
  `curl -X POST localhost:3000/mcp -d '{"method":"tools/call","params":{"name":"mnemonic_whoami"}}'`
  returns envelope with `supported_modes` field.
- Files to modify: `mcp/src/mcp.rs` (paywall + dispatch), `mcp/src/tools.rs`
  (whoami envelope, sign_memory input parsing, mode resolver).
- Files to read: `mcp/src/mcp.rs` (paywall gate inside `mcp_handler`),
  `mcp/src/tools.rs` (`whoami`, `sign_memory`, `sign_memory_inline`),
  `mcp/src/config.rs` (envelope fields), T1 output.

### Wave 3 — Delivery guarantee (mcp/)

**T3: Recall+verify round-trip on participate; demote-to-local on failure; no charge on failure; DoS guard**
- Description: Wrap the participate branch of `sign_memory_inline` in a
  delivery-confirmation step: after `solana.write_memo` returns, re-fetch
  the COSE bytes from Arweave (with exponential backoff up to the
  `MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS` wall-clock budget, default
  `15`) and run `verify_cose` plus an in-process `recall` against the
  fresh `content_hash`. On both-succeed: persist with
  `write_mode='participate'`, call `record_attestation_cost`, return
  `delivery_receipt`. On any failure: persist with `write_mode='local'`,
  skip `record_attestation_cost`, call `payment::refund_balance` with a
  reason that includes the demoted `attestation_id`, return JSON-RPC
  `-32011 DeliveryNotConfirmed`. Critical-section discipline: no `.await`
  while the SQLite mutex is held — two short scoped locks (one for the
  post-anchor persist, one for the demote+refund path).
  **Outcome-based DoS guard:** a small in-memory `RefundsBySubject`
  (`DashMap<api_key_hash, SlidingWindowCounter>`, 60-second window;
  **keyed by `api_key_hash`, NOT `owner_pubkey`** — Ed25519 keys are free
  to rotate but billable subjects aren't, so the latter is the right
  blast-radius). Bounded eviction: a background task running every
  `quota_evict_interval_secs` (default 30) drops entries whose window has
  been empty for the last 2× the window, keeping the map size proportional
  to *active* spenders, not lifetime cardinality. The counter is
  incremented in the refund branch and consulted at the *entry* of the
  participate path (BEFORE Arweave/Solana writes); on exceed
  (default 5/60s, both configurable) the request short-circuits to
  `-32011` with `data.kind: "DeliveryQuotaExceeded"`, spending zero chain
  fees. **DashMap shard discipline:** DashMap's per-shard lock is held
  briefly for the increment/read and never across an `.await` — same rule
  as the SQLite mutex (see Decision 8).
  Refund-itself failure extends the existing `payment_events` table with a
  new typed `event_kind = 'refund_failed'` row carrying ONLY
  `{api_key_hash, attestation_id, reason, occurred_at}` —
  **`api_key_hash` not the raw key** (credential-at-rest hygiene, CWE-312),
  no payload bytes, no `content_preview`, no embedding (PII allow-list
  pinned in the spec to avoid impl-time drift).
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`, `security-auditor`
- Verify-smoke: `cargo test -p mnemonic-mcp delivery_guarantee::happy_path`
  passes; `cargo test -p mnemonic-mcp delivery_guarantee::demotion_on_fetch_failure`
  passes with corrupted `httpmock` GET on the Arweave re-fetch;
  `cargo test -p mnemonic-mcp delivery_guarantee::quota_exceeded` passes
  (induced repeated demotions for one owner → 6th call short-circuits).
- Files to modify: `mcp/src/tools.rs` (sign_memory_inline participate branch,
  refetch helper), `mcp/src/payment.rs` (refund-reason wiring;
  `RefundsBySubject` counter; `payment_events` `event_kind` extension),
  `mcp/src/mcp.rs` (entry-point consultation of the quota counter),
  `mcp/src/config.rs` (new env var).
- Files to read: `mcp/src/tools.rs` (`sign_memory_inline` participate branch
  at the Arweave+Solana write block, plus `verify_cose` helper for
  reference), `mcp/src/payment.rs` (`refund_balance`, `payment_events`
  schema), `mcp/src/mcp.rs` (refund call site on tool-execution error,
  paywall entry), `core/src/arweave/mod.rs` (`new_for_test` httpmock
  pattern), T2 output.

### Wave 4 — Read paths (mcp/)

**T4: `verify` routes by stored `write_mode` column + tenant isolation on `find_by_tx`**
- Description: Replace the env-var branch in `verify` with a SQLite lookup
  of the row's `write_mode`. `local`-tagged rows route to `verify_local`;
  `participate`-tagged rows route to `verify_cose`/`verify_legacy_json`.
  Keep the env-var argument in the function signature for compatibility but
  ignore it for routing. Surface `write_mode` in the `recall` result
  envelope. **Tenant-isolation fix:** the `find_by_tx` lookup (and the
  routing query) must filter by the caller's `owner_pubkey` so cohabiting
  tenants in one DB cannot probe each other's rows via `verify` (closes the
  pre-existing gap identified in Decision 9).
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`, `security-auditor`
- Verify-smoke: `cargo test -p mnemonic-mcp verify_` and
  `cargo test -p mnemonic-mcp recall_` pass;
  `cargo test -p mnemonic-mcp verify_by_stored_mode::tenant_isolation` passes.
- Files to modify: `mcp/src/tools.rs` (verify routing, recall result shape),
  `core/src/storage/sqlite.rs` (`find_by_tx` + any sibling lookup gains an
  `owner_pubkey` filter).
- Files to read: `mcp/src/tools.rs` (`verify`, `verify_local`, `verify_cose`,
  `verify_legacy_json`, `recall`), `core/src/storage/sqlite.rs`
  (`find_by_tx`), patterns.md §"Tenant isolation via `owner_pubkey`",
  T1 output.

### Wave 5 — Docs

**T5: Retire "Never mix modes in one DB" rule; sync project-knowledge**
- Description: Update root `CLAUDE.md` "Storage modes" section to reflect
  per-request mode + S1 invariant. Update
  `.claude/skills/project-knowledge/references/patterns.md` "Storage modes"
  paragraph. Add a short "Mode dispatch" subsection in
  `.claude/skills/project-knowledge/references/architecture.md` Data Flow.
  No code changes.
- Skill: `documentation-writing`
- Reviewers: `code-reviewer`
- Files to modify: `CLAUDE.md`,
  `.claude/skills/project-knowledge/references/patterns.md`,
  `.claude/skills/project-knowledge/references/architecture.md`.
- Files to read: `work/modes-user-choice/user-spec.md`, T1-T4 outputs.

### Wave 6 — Audit (parallel; `reviewers: none`)

**A1: Code audit**
- Description: Holistic code-quality review of all feature code across
  `core/storage`, `mcp/tools`, `mcp/mcp`, `mcp/payment`. Write report to
  `work/modes-user-choice/logs/audit/code-audit.md`.
- Skill: `code-reviewing`
- Reviewers: none

**A2: Security audit**
- Description: OWASP Top 10 + tenant-isolation review across the feature
  surface; focus on the new error paths (`-32010`, `-32011`), payment-refund
  on delivery failure (no double-spend / no silent charge), and `write_mode`
  injection via input parsing. Write report to
  `work/modes-user-choice/logs/audit/security-audit.md`.
- Skill: `security-auditor`
- Reviewers: none

**A3: Test audit**
- Description: Coverage + test-quality review of unit + integration suites
  added in T1-T4. Verify the compatibility-regression block exists. Write
  report to `work/modes-user-choice/logs/audit/test-audit.md`.
- Skill: `test-master`
- Reviewers: none

### Wave 7 — Final

**F1: Pre-deploy QA**
- Description: Run `cargo test --workspace --no-fail-fast`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`.
  Verify all acceptance criteria from user-spec invariants (free local,
  paid participate, whoami envelope, UnsupportedMode error, delivery
  guarantee, compatibility with deferred-signing path). No live-environment
  step in this feature — deploy and post-deploy gates are run as part of
  the broader release, not this feature's scope.
- Skill: `pre-deploy-qa`
- Reviewers: none

## Acceptance Criteria

All AC are testable in CI without a live environment. Each maps to one or
more user-spec invariants (anchor in brackets).

1. **Free local against `full` server.** `sign_memory { mode: "local" }`
   against a `STORAGE_MODE=full + PAYMENT_MODE=x402` server returns a
   `local:`-prefixed pair of synthetic tx ids, no Arweave/Solana writes,
   row tagged `write_mode='local'`, no `attestation_costs` row, no payment
   header required. [user-spec invariant: "Личная память бесплатна всегда"]
2. **Legacy backward compatibility.** `sign_memory` with no `mode` field
   produces a response with identical field shape to today's pre-change
   path (golden-fixture asserted in T2). [user-spec invariant: "Совместимость
   с shipped `work/chrome-extension/`"]
3. **Unsupported-mode typed error.**
   `sign_memory { mode: "participate" }` against a `STORAGE_MODE=local`
   server returns JSON-RPC `-32010 UnsupportedMode` with
   `data.supported = ["local"]`, no row written, no charge. Malformed
   `mode` values (`null`, non-string, empty, case-variant, unknown) return
   `-32602 InvalidParams`. [user-spec invariant: "типизированная ошибка,
   не тихий downgrade"]
4. **`whoami` envelope per deploy variant.** Returns
   `supported_modes`, `default_mode`, `participate_cost` correctly shaped
   for: local-only (`participate_cost: null`), self-operator
   (`participate_cost.amount_cents: 0, payment_methods: []`), hosted-x402
   (`amount_cents > 0, payment_methods: ["x402", …]`). [user-spec
   invariant: "Discoverability через `whoami`"]
5. **Delivery happy path.** Successful `participate` write returns a
   success envelope including `delivery_receipt { arweave_tx, solana_tx,
   recall_verified_at }`; row tagged `write_mode='participate'`;
   `attestation_costs` row written. [user-spec invariant: "«Доставлено»
   = заякорено И подтверждено через recall"]
6. **Delivery failure path.** Induced Arweave re-fetch corruption demotes
   the row to `write_mode='local'`, skips `attestation_costs`, refunds
   the reserved balance (post-call balance equals pre-call balance), and
   returns `-32011 DeliveryNotConfirmed`. [user-spec invariant: "Пока
   артефакт не перечитан … запись остаётся local и плата не берётся"]
7. **Mixed-mode coexistence.** A single owner can hold `local` and
   `participate` rows in the same DB; `recall` returns both and tags each
   with its stored `write_mode`. [user-spec invariant: "сосуществуют у
   одного пользователя в одной БД, тегируясь `write_mode`"]
8. **Tenant isolation on verify.** Caller A's `verify` against caller B's
   `solana_tx` returns "not found" with no leaked content/hash/signer.
   [project pattern: "Tenant isolation via `owner_pubkey`"]
9. **Docs alignment.** Root `CLAUDE.md` no longer carries the literal
   string "Never mix in one DB"; `patterns.md` storage-modes paragraph
   matches the new per-request semantics. [user-spec invariant:
   "ретайрится"]

## Agent Verification Plan

### Tools required
None. CI invocations only: `cargo test --workspace --no-fail-fast`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`.

### Verification approach
All 9 acceptance criteria map to specific tests (unit or `mcp/tests/*`
integration) listed in §"Testing strategy". The pre-deploy QA task (F1)
runs the suite and explicitly cross-checks each AC against its
corresponding test name. Post-deploy verification is out of scope for
this feature — it travels with the next general MCP-server release.

## Risk & mitigations

- **Trait signature change ripples through internal callsites.** T1 starts
  by grepping `\.save_attestation(` across the workspace to enumerate
  callsites, then updates them all in one PR. CI catches anything missed.
- **Backfill heuristic for legacy rows.** `solana_tx LIKE 'local:_%'`
  (strict — at least one char after the colon) is collision-safe against
  real Solana signatures (base58 excludes lowercase `l`, and `:` is not in
  the base58 alphabet at all). Risk: a deployment with hand-modified rows.
  Mitigation: idempotent migration can be re-run; column defaults to
  `'participate'` (safer when in doubt — paid implies trustworthy).
- **DoS amplification via induced delivery failure.** Without mitigation a
  caller could trigger participate writes that always fail at re-fetch,
  bleeding operator margin (the operator already paid Arweave + Solana
  fees) while being fully refunded. Mitigation:
  (i) **Wall-clock retry budget** on the Arweave re-fetch in T3 — default
  `MNEMONIC_DELIVERY_REFETCH_TIMEOUT_SECS=15` (configurable per operator),
  retried with exponential backoff up to the budget. Sized against
  Arweave's documented eventual-consistency window (seconds to low tens of
  seconds), not a fixed two-attempt cap. Inside the budget the request
  blocks; on budget exhaustion the row is demoted and the refund fires.
  (ii) **Outcome-based per-billable-subject counter consulted at
  participate entry** — a small in-memory `RefundsBySubject`
  (`DashMap<api_key_hash, SlidingWindowCounter>` with a 60-second sliding
  window) is incremented in the refund branch of T3 and *read* at the top
  of the participate path in `mcp_handler` **before** any chain write.
  **Keyed by `api_key_hash`, not `owner_pubkey`** — Ed25519 keys rotate
  freely, billable subjects don't; keying on the wrong identifier would
  let a caller bypass the quota by minting a new identity per request.
  Bounded eviction: idle entries (empty window for 2×window) are dropped
  on a configurable interval (`quota_evict_interval_secs`, default 30) so
  map size tracks *active* spenders, not lifetime cardinality. If a
  caller exceeds the threshold (default 5 demotions in 60 s, both env-
  configurable for empirical tuning) the next `participate` request
  short-circuits to `-32011` with `data.kind: "DeliveryQuotaExceeded"`,
  **without** spending Arweave/Solana fees. `tower_governor` is *not*
  used because it rate-limits requests, not outcomes. The new counter
  lives in `mcp/src/payment.rs` next to the existing payment state.
  (iii) **Metrics** — Prometheus counter
  `mnemonic_delivery_not_confirmed_total{stage}` (label set:
  `stage ∈ {refetch, verify, recall}`; **no `owner_pubkey` or
  `api_key_hash` label** — high-cardinality anti-pattern). Per-tenant
  detail goes to structured log lines (`tracing::warn!(api_key_hash=…,
  owner_pubkey=…, stage=…)`) for ops investigation without TSDB blow-up.
  Decision call-out in T3.
  (iv) **SLO note for operators.** The 15s refetch budget and 5/60s quota
  are *starting* defaults; operators should set them empirically from
  observed legitimate-traffic demotion rate. Both are env vars so they
  can be tuned without a redeploy of behavior.
- **`.await` discipline.** T3 inserts new awaits (Arweave re-fetch,
  `verify_cose`, `recall`) into the existing scoped-lock block. Mitigation:
  per Decision 8, two short critical sections — neither holds the SQLite
  lock across an `.await`. Code-audit (A1) checks this explicitly.
- **Refund audit-trail correctness.** Mitigation: T3 includes the demoted
  `attestation_id` in the refund reason and writes a structured audit row
  on refund-itself failure (Decision 7). Security audit (A2) reviews.
- **Tenant isolation on cohabiting modes.** Now that one DB carries many
  tenants × two modes, `find_by_tx` and the new routing lookup MUST scope
  by `owner_pubkey`. Mitigation: T4 closes the pre-existing gap as part of
  the routing change (Decision 9); regression test in
  `mcp/tests/verify_by_stored_mode.rs`.
- **Backward compatibility with shipped extension.** Mitigation: T2 keeps
  the `mode`-absent path on the legacy env-var fallback verbatim and
  pins the response shape behind a golden-fixture test; T1's migration
  helper is idempotent so DBs that haven't seen the new column work after
  one boot; existing `deferred_sign_flow.rs` and `sign_callback.rs` test
  *assertions* must pass unchanged (internal helper callsites are updated
  for the new `WriteMode` parameter, as called out in the compat-regression
  paragraph of the testing strategy).
