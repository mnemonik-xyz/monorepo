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
encodes the consequence as a hard rule: *"Mode is set at startup, not per-call.
Never mix in one DB."* This spec deliberately revisits that rule.

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
    `WriteMode`, branch on resolved mode (today branches on `storage_mode`
    string at lines `332` and `367`); on `Participate`, run the
    recall+verify round-trip before persisting "delivered" state.
  - `whoami` (line `27`) — return envelope.
  - `verify` (line `419`) — route by stored `write_mode` column.
- `mcp/src/mcp.rs:424` — paywall gate. Today: `payment_mode != "none" &&
  storage_mode != "local"`. New: gate on resolved `WriteMode::Participate`.
  Requests without `mode` field continue to fall back to env-var-driven
  legacy behavior (compat).
- `mcp/src/payment.rs` — no API change; only callsite moves. `Local` writes
  never reach `check_payment` / `record_attestation_cost`.
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
   recall"]. Uses existing `verify_cose` (`tools.rs:479`) and the SQLite
   cosine recall path — no new primitive.
7. **On delivery failure: row demoted to `local`, no charge.** [supports
   user-spec "плата не берётся" + "запись остаётся local"]. Implementation:
   wrap the participate path in a transaction-like rollback that flips
   `write_mode` and skips `record_attestation_cost`; the reserved
   balance/x402 payment is released via the existing `payment::refund` path
   (currently used by error returns in `mcp.rs`).
8. **[TECHNICAL] Trait signature change is breaking inside the workspace.**
   `AttestationStore::save_attestation` gains a `WriteMode` parameter. All
   11 internal callsites in `core/src/storage/sqlite.rs` (test helpers) plus
   `mcp/src/tools.rs` and `mcp/src/api.rs::sign_callback` must be updated in
   the same task. No external crates depend on this trait.

## User-Spec Deviations

None. All tech-spec decisions trace to user-spec invariants (anchor IDs in
each Decision above).

## Data model

### Schema migration

Idempotent helper `migrate_write_mode_column` (mirrors
`migrate_owner_pubkey_columns` in `core/src/storage/sqlite.rs:148`):

```sql
-- 1. Add column with safe default for fresh schemas.
ALTER TABLE attestations
  ADD COLUMN write_mode TEXT NOT NULL DEFAULT 'participate';

-- 2. Backfill legacy rows based on tx-id shape.
UPDATE attestations
   SET write_mode = 'local'
 WHERE solana_tx LIKE 'local:%';

-- 3. Index for filtered recall + audit queries.
CREATE INDEX IF NOT EXISTS idx_attestations_write_mode
  ON attestations(owner_pubkey, write_mode);
```

`DEFAULT 'participate'` is the conservative choice: a row that existed under
the legacy global `STORAGE_MODE=full` operator was, by definition, a paid
participate write. The `LIKE 'local:%'` backfill catches the
`storage_mode=local` rows independently.

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
    "recall_verified_at": "2026-06-01T12:34:56Z"
  }
}
```

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
- `migrate_write_mode_column` — fresh-schema path, legacy-rows backfill path,
  idempotency (run twice, same result).
- `whoami` envelope shape per config: `STORAGE_MODE=local` →
  `supported_modes=["local"]`; `STORAGE_MODE=full + PAYMENT_MODE=none` →
  `participate_cost.amount_cents=0`; `full + PAYMENT_MODE=x402` →
  `participate_cost.payment_methods=["x402"]`.
- `parse_mode` request parsing — accepts `"local"`, `"participate"`, missing
  (→ env-var fallback), unknown (→ JSON-RPC `InvalidParams`).
- `paywall_gate(WriteMode)` — pure function returning whether to charge;
  `WriteMode::Local` always false.

### Integration (Rust, `tests/`)
- `tests/modes_per_request.rs` (new) — drive the MCP HTTP/stdio dispatcher
  end-to-end:
  - `sign_memory { mode: "local" }` against a `STORAGE_MODE=full` server →
    free, no Arweave write, row tagged `local`.
  - `sign_memory { mode: "participate" }` against `STORAGE_MODE=local` server
    → `UnsupportedMode` (-32010), no row written.
  - `sign_memory { /* no mode */ }` legacy path → identical bytes/behavior to
    today's tests (regression guard for shipped extension).
  - `whoami` envelope shape per `Config`.
- `tests/delivery_guarantee.rs` (new) — uses the existing `arlocal` +
  `solana-test-validator` harness:
  - happy path: anchor → recall+verify succeeds → row `participate`,
    `delivery_receipt` returned, charge recorded.
  - induced failure: stub the Arweave fetch to return wrong bytes → row
    demoted to `local`, no `attestation_costs` row, JSON-RPC error
    `DeliveryNotConfirmed`.

### Compatibility regression
The shipped extension's Cloud-tier exercises the deferred-signing path
(`sign_memory_deferred`, `/api/sign-callback`). All existing tests in
`tests/sign_memory_deferred_*` and `tests/sign_callback_*` must pass
unchanged — the new `mode` field is parsed once at the top of `sign_memory`
and the deferred branch (`jwt_sub.is_some()`) is entered before any
mode-specific code path, so its behavior is byte-identical when `mode` is
absent.

### Not covered by this feature
- Browser-side behavior — out of scope (kept compatible, not modified).
- Pluggable signer / chain-pluggable anchor (issue #29).
- Wallet-connect UX (browser-side, future).

## Implementation Tasks

### Wave 1 — Foundation (core/)

**T1: WriteMode type + DB schema migration + save_attestation signature**
- Description: Add `WriteMode { Local, Participate }` enum in
  `core/src/storage/mode.rs`. Extend `AttestationStore::save_attestation`
  and `SqliteStore::save_attestation` with a `WriteMode` parameter; persist
  it as a new `write_mode TEXT NOT NULL` column. Add idempotent
  `migrate_write_mode_column` helper that adds the column with default
  `'participate'` then backfills `'local'` for legacy rows whose `solana_tx`
  starts with `local:`. Update all internal callsites in
  `core/src/storage/sqlite.rs` tests.
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`
- Verify-smoke: `cargo test -p mnemonic-core storage::` passes; run
  `cargo test -p mnemonic-core migrate_write_mode_` to exercise both
  fresh-schema and legacy-DB paths.
- Files to modify: `core/src/storage/mode.rs` (new), `core/src/storage/mod.rs`,
  `core/src/storage/sqlite.rs`, `core/src/storage/traits.rs`, `core/src/lib.rs`.
- Files to read: `core/src/storage/sqlite.rs:148-280` (migration helpers),
  `core/src/storage/sqlite.rs:388-430` (save_attestation), user-spec.md.

### Wave 2 — API surface (mcp/)

**T2: Per-request `mode` field + UnsupportedMode error + paywall reframing + whoami envelope**
- Description: Add optional `mode` field to `mnemonic_sign_memory` input.
  Resolve to `WriteMode` (default = legacy env-var when absent; `local` when
  explicit). Move the paywall gate at `mcp/src/mcp.rs:424` to fire only when
  resolved mode is `Participate`. Extend `whoami` output with envelope
  fields. Emit JSON-RPC error `-32010 UnsupportedMode` when a caller
  requests a mode outside `supported_modes`. Thread resolved `WriteMode`
  into `sign_memory_inline` and `save_attestation`. Update all callsites in
  `tools.rs` that today branch on `storage_mode: &str`.
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`
- Verify-smoke: `cargo test -p mnemonic-mcp` passes; quick HTTP probe —
  `curl -X POST localhost:3000/mcp -d '{"method":"tools/call","params":{"name":"mnemonic_whoami"}}'`
  returns envelope with `supported_modes` field.
- Files to modify: `mcp/src/mcp.rs` (paywall + dispatch), `mcp/src/tools.rs`
  (whoami envelope, sign_memory input parsing).
- Files to read: `mcp/src/mcp.rs:410-440`, `mcp/src/tools.rs:25-100`,
  `mcp/src/tools.rs:280-400`, `mcp/src/config.rs:30-95`, T1 output.

### Wave 3 — Delivery guarantee (mcp/)

**T3: Recall+verify round-trip on participate; demote-to-local on failure; no charge on failure**
- Description: Wrap the participate branch of `sign_memory_inline` (today's
  Arweave/Solana write block at `tools.rs:332-367`) in a delivery confirmation
  step: after `solana.write_memo` returns, re-fetch the COSE bytes from
  Arweave and run `verify_cose` plus an in-process `recall` against the
  fresh `content_hash`. On both-succeed: persist `write_mode='participate'`
  and `record_attestation_cost`; return `delivery_receipt`. On any failure:
  persist `write_mode='local'`, skip `record_attestation_cost`, release the
  reserved payment via the existing refund path, return JSON-RPC
  `-32011 DeliveryNotConfirmed`. Add `delivery_receipt` to the success
  envelope.
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`, `security-auditor`
- Verify-smoke: integration test `tests/delivery_guarantee.rs::happy_path`
  passes; `tests/delivery_guarantee.rs::demotion_on_fetch_failure` passes
  with stubbed Arweave returning wrong bytes.
- Files to modify: `mcp/src/tools.rs` (sign_memory_inline participate branch),
  `mcp/src/payment.rs` (refund callsite, no API change).
- Files to read: `mcp/src/tools.rs:280-405`, `mcp/src/payment.rs`,
  `mcp/src/tools.rs:479` (verify_cose), T2 output.

### Wave 4 — Read paths (mcp/)

**T4: `verify` routes by stored `write_mode` column**
- Description: Replace the env-var branch in `mcp/src/tools.rs::verify`
  (line `419`: `if storage_mode == "local"`) with a SQLite lookup of the
  row's `write_mode`. `local`-tagged rows route to `verify_local`;
  `participate`-tagged rows route to the existing
  `verify_cose`/`verify_legacy_json` path. Keep the env-var signature for
  backward compatibility but ignore it for routing. `recall` already returns
  the row's stored fields; surface `write_mode` in the recall result envelope.
- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`
- Verify-smoke: `cargo test -p mnemonic-mcp verify_` and
  `cargo test -p mnemonic-mcp recall_` pass.
- Files to modify: `mcp/src/tools.rs` (verify routing, recall result shape).
- Files to read: `mcp/src/tools.rs:407-630`, T1 output.

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

## Agent Verification Plan

This feature is purely server-side and ships its acceptance behind
`cargo test`. No MCP tools required beyond standard CI invocations.
Post-deploy verification not in scope — it travels with the next general
release of the MCP server.

- **Tools required:** none (only `cargo test`, `cargo clippy`, `cargo fmt`).
- **Acceptance criteria** (all from user-spec invariants):
  1. `sign_memory { mode: "local" }` against a `full` server returns free,
     no Arweave/Solana tx, row `write_mode='local'`.
  2. `sign_memory` with no `mode` field against any server returns the same
     bytes as today's legacy path (regression).
  3. `sign_memory { mode: "participate" }` against a `local`-only server
     returns JSON-RPC `-32010 UnsupportedMode`.
  4. `whoami` returns the envelope; envelope reflects `Config` correctly
     for all three deploy variants (local-only, self-operator, hosted-x402).
  5. Successful participate write returns `delivery_receipt`; failed one
     returns `-32011 DeliveryNotConfirmed` with the row demoted and no
     `attestation_costs` row.
  6. CLAUDE.md no longer states "Never mix modes in one DB".

## Risk & mitigations

- **Trait signature change ripples through ~15 callsites.** Mitigation:
  T1 bundles the signature change with all internal callsite updates in
  one task; CI catches anything missed.
- **Backfill heuristic for legacy rows.** `solana_tx LIKE 'local:%'` is
  reliable for rows written via the `STORAGE_MODE=local` code path
  (`tools.rs:332-334` produces this exact prefix). Risk: a deployment
  that hand-modified rows. Mitigation: idempotent migration helper can be
  re-run after manual fix-ups; the new column defaults to `'participate'`
  which is the safer assumption (paid → trustworthy until shown otherwise).
- **Refund correctness on delivery failure.** Mitigation: T3 reuses the
  existing payment refund path that is exercised by current error-return
  tests; security audit (A2) reviews specifically this case.
- **Backward compatibility with shipped extension.** Mitigation: T2 keeps
  the `mode`-absent path on the legacy env-var fallback verbatim; T1
  defaults the migration column so DBs without the new column work after
  one schema migration; existing deferred-signing tests must pass
  unchanged (compat regression block in test strategy).
