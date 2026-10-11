# Tech spec: receipts (a signed ledger of agent memory)

Status: draft for owner review, 2026-10-11. Nothing in this spec is implemented.
Every capability below is **planned** unless a line says "available now".

Related: `docs/comparisons.md` (memory-only systems table), `work/verifiable-trajectories/`
(`STEP_V1`, the existing chained artifact), `docs/recovery-checkpoints.md`.
User story and rationale: `user-spec.md`. Open questions and decisions: `decisions.md`.

## 1. Summary

One sentence: **mem0 remembers; Mnemonic proves what it remembered.**

A receipt is a small signed record of one memory mutation (`ADD`, `UPDATE`, `DELETE`,
`RESET`) in an external memory engine. A receipt holds hashes, identifiers and chain links.
It never holds memory text. Receipts form a hash chain per ledger and a second chain per
scope. A checkpoint commits a range of receipts to one Merkle root. Anyone who has the
public key can verify an exported ledger offline, without the operator and without the engine.

`Ledger.wrap(memory)` attaches a receipt writer to a mem0 `Memory` or `AsyncMemory` (first)
and to `MemoryClient` (later). The mem0 API does not change.

Out of scope for v1: graph-memory mutations (`mem0.graphs`), receipts for reads
(`search`, `get`), key rotation, the hosted notary, and engines other than mem0.

## 2. Current state (verified in code, 2026-10-11)

mem0 2.2.1 (`mem0ai`, fork `mnemonik-dev/mem0` at `b7ad69a`):

| Fact | Source |
|---|---|
| Every self-hosted mutation ends in `SQLiteManager.add_history` or `batch_add_history`. | `mem0/memory/main.py:1091,1096,1995,2094,2126`; `mem0/memory/storage.py:150-225` |
| The additive `add` path inserts a batch into the vector store, then calls `batch_add_history`. On failure it falls back to one `add_history` call per record. | `main.py:1076-1098` |
| `infer=False` adds and procedural memories go through `_create_memory`. | `main.py:881-905`, `2007-2050` |
| `delete_all` lists memories for a scope and calls `_delete_memory` for each one. | `main.py:1904-1960` |
| `reset()` calls `db.reset()`, `db.close()` and then **replaces** `self.db` with a new `SQLiteManager`. | `main.py:2144-2156`, `3865-3882` |
| `AsyncMemory` calls the same `self.db` methods through `asyncio.to_thread`. | `main.py:2772-2780`, `3692-3700`, `3814`, `3848` |
| A history row holds `memory_id, old_memory, new_memory, event, created_at, updated_at, is_deleted, actor_id, role`. It does not hold `user_id`, `agent_id` or `run_id`. | `storage.py:108-120`, `150-162` |
| Scope keys live in the vector payload (`user_id`, `agent_id`, `run_id`). Identity keys cannot change on update. | `main.py:2074`, `2107`, `2124` |
| The mem0 content hash is MD5 of the text. It is not signed. | `main.py:1984`, `2077` |
| Timestamps are UTC ISO 8601 with microseconds. | `main.py:1987`, `2080`, `2122` |
| The platform client posts to `/v3/memories/add/` and returns the API JSON. `update`, `delete`, `delete_all`, `batch_update` and `batch_delete` exist. | `mem0/client/main.py:303-340`, `463`, `504`, `535`, `692`, `719` |
| No audit, provenance, tamper or signature code exists in `mem0/`. | `grep -rn -i "audit\|provenance\|tamper\|signature" mem0/` gives 0 hits |

monorepo (`mnemonik-xyz/monorepo` at `fe06662`):

| Fact | Source |
|---|---|
| COSE_Sign1 signing: protected `alg = EdDSA`, `content_type = application/cbor`; unprotected `kid` = base58 Ed25519 public key. | `core/src/codec/sign.rs:53-90` |
| Verification parses `kid` as a Solana `Pubkey` and checks the 64-byte signature over `Sig_structure`. | `core/src/codec/sign.rs:103-140` |
| Content hash = blake3 over canonical CBOR, lowercase hex. | `core/src/codec/hash.rs:17-28` |
| Canonical CBOR: top-level keys follow `cbor_field_order`; null or missing optional fields are omitted; nested map keys are sorted; non-integer numbers become text; **every string that parses as RFC 3339 becomes CBOR tag 1 with epoch seconds**. | `core/src/codec/canonical.rs:1-13`, `26-56`, `66-110` |
| `STEP_V1` is the existing chained artifact: `seq` and a nullable `prev_hash` inside the signed payload. | `core/src/codec/schema.rs:423-460` |
| Merkle: `commitment_root`, `prove`, `verify`, `parse_hex32`, `to_hex32`. Leaves are 32-byte hashes. | `core/src/merkle.rs:36-239` |
| CLI identity file: `~/.mnemonic/identity.json` = `{secret: number[64], pubkey_base58}`. | `packages/cli/src/config.ts:9` |
| Golden fixture emitter prints `{name, input_hex, canonical_cbor_hex, cose_envelope_hex, keypair_secret_hex}`. | `core/tests/golden_fixtures.rs:1-30` |

## 3. Decisions

- **D-1 Hashes only.** Memory text, metadata values and scope values never enter a receipt.
  A receipt is safe to export, publish or send to a notary.
- **D-2 Hook point: the history writer.** A `ReceiptingHistory` proxy replaces `Memory.db`
  and wraps `add_history` and `batch_add_history`. Scope comes from a `contextvars.ContextVar`.
  Thin wrappers around the public methods `add`, `update`, `delete` and `delete_all` set it.
  Rationale: the history writer is the only point that every mutation path crosses (§2).
  Wrapping `_create_memory` alone misses the batch path. Wrapping the public API alone
  never sees the old text.
- **D-3 Two chain links.** Each receipt carries `prev` (ledger order) and `scope_prev`
  (per-scope order). A per-scope export verifies on its own. Checkpoints cover the full ledger.
- **D-4 `content_hash` = blake3 of the UTF-8 bytes of the memory text.** No normalisation.
  `engine_hash` keeps the mem0 MD5 for cross-reference only.
- **D-5 Receipt id = blake3 of the full COSE_Sign1 bytes** (exact-envelope digest, the same
  rule as recovery pins). `prev`, `scope_prev` and Merkle leaves use receipt ids.
- **D-6 Timestamps are integers** (unix milliseconds, UTC). `RECEIPT_V1` and `CHECKPOINT_V1`
  contain no RFC 3339 string. Rationale: the canonicaliser turns RFC 3339 strings into tag-1
  seconds, and Python 3.9 `fromisoformat` does not match chrono. Integers remove the risk.
- **D-7 `scope_id` = blake3 of plain deterministic CBOR** (RFC 8949 §4.2.1) of the scope map
  `{key: text}`. It does not use the artifact canonicaliser. Rationale: a scope value that
  looks like a timestamp must not change its encoding.
- **D-8 Fail closed, but never corrupt the engine.** Default `on_error="raise"`. The proxy never
  raises after the inner history write succeeded, because mem0 would then run its fallback and
  write duplicate history rows (`main.py:1090-1098`). The proxy stores the failure, and the
  public wrapper raises `ReceiptError` after the engine call returns. `on_error="warn"` logs.
- **D-9 Store = SQLite with append-only triggers. Interchange = JSON Lines.**
- **D-10 Pure-Python signer and verifier** (`cbor2`, `blake3`, `cryptography`, `base58`).
  Rust `core` stays the reference verifier. Golden fixtures prove byte parity.
- **D-11 No network call in the write path.** Anchoring a checkpoint is a separate command.
- **D-12 Engine pin `mem0ai>=2.2,<3`.** `wrap` checks the hook surface at run time. It refuses
  an unknown engine unless `allow_unverified_engine=True`.

## 4. `RECEIPT_V1`

### 4.1 Fields

| Field | CBOR type | Presence | Value |
|---|---|---|---|
| `type` | text | required | `"receipt"` |
| `schema_version` | int | required | `1` |
| `ledger_id` | text | required | UUID v4 of the ledger. Fixed when the ledger is created. |
| `seq` | int | required | 0-based position in the ledger. |
| `prev` | text | nullable | Receipt id of `seq - 1`. Absent when `seq == 0`. |
| `scope_id` | text | required | Hex blake3 of the scope map (D-7). `RESET` and unscoped receipts use the id of the empty map. |
| `scope_seq` | int | required | 0-based position in the chain of this `scope_id`. |
| `scope_prev` | text | nullable | Receipt id of the previous receipt with the same `scope_id`. Absent when `scope_seq == 0`. |
| `engine` | text | required | `"<name>-<flavour>/<version>"`, for example `"mem0-oss/2.2.1"`, `"mem0-platform/2.2.1"`. |
| `op` | text | required | `ADD`, `UPDATE`, `DELETE` or `RESET`. The verifier accepts other upper-case tokens from other engines. |
| `memory_id` | text | nullable | The engine memory id. Absent for `RESET`. |
| `content_hash` | text | nullable | Hex blake3 of the text **after** the op. Absent for `DELETE` and `RESET`. |
| `prev_content_hash` | text | optional | Hex blake3 of the text **before** the op. Present for `UPDATE` and `DELETE` when the engine reports the old text. |
| `engine_hash` | text | optional | The engine's own hash of the new text (mem0: MD5 hex). Informational. |
| `engine_actor` | text | optional | `actor_id` from the engine history row. |
| `engine_role` | text | optional | `role` from the engine history row. |
| `engine_ts` | int | optional | Engine time of the op (`updated_at`, else `created_at`) as unix ms UTC. |
| `ts` | int | required | Receipt creation time, unix ms UTC, from the signer's clock. |
| `producer` | text | required | Base58 Ed25519 public key of the signer. Must equal the COSE `kid`. |

`cbor_field_order` (append-only, never reorder):

```
type, schema_version, ledger_id, seq, prev, scope_id, scope_seq, scope_prev,
engine, op, memory_id, content_hash, prev_content_hash, engine_hash,
engine_actor, engine_role, engine_ts, ts, producer
```

`bytes_fields`: none. All hashes are lowercase hex text, like `content_hash` in `MEMORY_V1`.

### 4.2 Rules per op

| `op` | `memory_id` | `content_hash` | `prev_content_hash` | scope |
|---|---|---|---|---|
| `ADD` | engine id | blake3(new text) | absent | from the `add` arguments |
| `UPDATE` | engine id | blake3(new text) | blake3(old text) | from the stored payload (pre-read) |
| `DELETE` | engine id | absent | blake3(old text) | from the stored payload (pre-read) |
| `RESET` | absent | absent | absent | empty map |

Notes:

- A metadata-only mem0 update (`data=None`) keeps the text. The receipt then has
  `content_hash == prev_content_hash`. That is correct: the engine reported an `UPDATE`.
- One batch `add` gives one receipt per persisted record. The receipts of one batch are
  written in one store transaction and get consecutive `seq` values.
- The scope map is open. mem0 uses `user_id`, `agent_id`, `run_id`. The platform adapter may
  add `app_id`. Keys are sorted at encoding time, so the key set does not change the rule.

### 4.3 Scope id (D-7)

```
scope_bytes = deterministic_cbor({ "agent_id": "a1", "user_id": "u1" })   # sorted keys, text values
scope_id    = hex(blake3(scope_bytes))
empty_scope = hex(blake3(deterministic_cbor({})))  = hex(blake3(0xa0))
```

The raw scope map stays in the local store (`scopes` table, §6.7). An export can include it
on request. A verifier that has the raw scope recomputes `scope_id` and checks it.

### 4.4 Envelope and identifiers

- Payload = canonical CBOR of the fields above, by the rules in `core/src/codec/canonical.rs`.
- Envelope = COSE_Sign1 exactly as `sign_cose` builds it (§2): protected `alg = EdDSA`,
  `content_type = application/cbor`; unprotected `kid` = base58 public key; Ed25519 over
  `Sig_structure` with an empty external AAD.
- `payload_hash` = hex blake3 of the payload bytes (what `verify_artifact` reports as `content_hash`).
- `receipt_id` = hex blake3 of the full envelope bytes (D-5). This is the value in `prev`,
  `scope_prev`, the Merkle leaves and the export lines.

### 4.5 Example (JSON view of three payloads)

```json
{"type":"receipt","schema_version":1,"ledger_id":"6f1c…","seq":0,
 "scope_id":"9b2e…","scope_seq":0,"engine":"mem0-oss/2.2.1","op":"ADD",
 "memory_id":"c3d4…","content_hash":"a1b2…","engine_hash":"5d41…",
 "engine_ts":1760176800123,"ts":1760176800130,"producer":"7f3a…"}

{"type":"receipt","schema_version":1,"ledger_id":"6f1c…","seq":1,"prev":"<id of seq 0>",
 "scope_id":"9b2e…","scope_seq":1,"scope_prev":"<id of seq 0>","engine":"mem0-oss/2.2.1",
 "op":"UPDATE","memory_id":"c3d4…","content_hash":"e5f6…","prev_content_hash":"a1b2…",
 "engine_hash":"8e9f…","engine_ts":1760176860000,"ts":1760176860004,"producer":"7f3a…"}

{"type":"receipt","schema_version":1,"ledger_id":"6f1c…","seq":2,"prev":"<id of seq 1>",
 "scope_id":"<empty_scope>","scope_seq":0,"engine":"mem0-oss/2.2.1","op":"RESET",
 "ts":1760177000000,"producer":"7f3a…"}
```

## 5. `CHECKPOINT_V1`

A checkpoint commits the receipts `from_seq..=to_seq` to one Merkle root. Checkpoints form
their own chain. A checkpoint is the only artifact that leaves the machine when the owner
anchors it (D-11). It is about 200 bytes.

| Field | CBOR type | Presence | Value |
|---|---|---|---|
| `type` | text | required | `"checkpoint"` |
| `schema_version` | int | required | `1` |
| `ledger_id` | text | required | Same as the receipts. |
| `idx` | int | required | 0-based checkpoint number. |
| `prev` | text | nullable | Id of checkpoint `idx - 1`. Absent when `idx == 0`. |
| `prev_anchor` | text | optional | Locator of the previous anchored checkpoint, for example `arweave:<tx>`. |
| `from_seq` | int | required | First receipt covered. |
| `to_seq` | int | required | Last receipt covered. |
| `count` | int | required | `to_seq - from_seq + 1`. |
| `root` | text | required | Hex `commitment_root` over the receipt ids in `seq` order (`core/src/merkle.rs:119`). |
| `head` | text | required | Receipt id at `to_seq`. |
| `ts` | int | required | Unix ms UTC. |
| `producer` | text | required | Base58 public key. Must equal `kid`. |

`cbor_field_order`: `type, schema_version, ledger_id, idx, prev, prev_anchor, from_seq, to_seq,
count, root, head, ts, producer`. Checkpoint id = blake3 of the envelope bytes.

Leaves are the 32-byte receipt ids (`parse_hex32`). The tree uses the existing `leaf_hash` and
node rules, so the Rust `verify` function checks a Python-built proof unchanged.

## 6. `Ledger.wrap` design

Package: `mnemonik` on PyPI (name to confirm, `decisions.md` Q-1). Source: `python/mnemonik/`
in this repository. Python 3.9 to 3.12, the same range as mem0. Dependencies: `cbor2`,
`blake3`, `cryptography`, `base58`. `mem0ai` is an optional extra (`mnemonik[mem0]`).

### 6.1 Components

```
python/mnemonik/
├── codec.py        canonical CBOR (artifact rules), deterministic CBOR (scope rules), blake3
├── cose.py         sign_cose / verify_cose, identical header layout to core/src/codec/sign.rs
├── keys.py         Signer: identity.json read/create, seed -> Ed25519, base58 public key
├── schema.py       RECEIPT_V1, CHECKPOINT_V1 field orders and validation
├── store.py        LedgerStore: SQLite tables, append-only triggers, head state, transactions
├── ledger.py       Ledger: open / wrap / unwrap / export / checkpoint / audit / receipts
├── engines/
│   ├── base.py     EngineAdapter protocol: wrap(obj), unwrap(obj), engine_label
│   ├── mem0_oss.py ReceiptingHistory proxy + public-method wrappers + ContextVar scope
│   └── mem0_platform.py  planned (§6.11)
├── verify.py       offline verifier for JSON Lines exports
└── cli.py          mnemonik verify | export | checkpoint | audit | keygen
```

### 6.2 `wrap` procedure (mem0 OSS)

1. Detect the engine: `isinstance(obj, mem0.Memory | mem0.AsyncMemory)`. Read `mem0.__version__`.
2. Check the hook surface: `obj.db` has `add_history` and `batch_add_history` with the
   parameter names in §2. If not, raise `UnsupportedEngine` (D-12).
3. Open or create the ledger store. Load `ledger_id`, `producer`, head `seq` and the last
   receipt id per `scope_id`.
4. Replace `obj.db` with `ReceiptingHistory(inner=obj.db, ledger=self)`.
5. Replace the bound methods `add`, `update`, `delete`, `delete_all`, `reset` on the instance
   (not on the class) with wrappers. Async variants get `async` wrappers.
6. Set `obj.ledger = self`. Return `obj`.

`unwrap` restores the original `db` and bound methods.

### 6.3 Emission sequence per op

```
add(messages, user_id=.., agent_id=.., run_id=..)
  wrapper: scope = trimmed non-empty ids   -> _SCOPE.set(scope)   # same trim rule as mem0
  mem0.add -> ... -> vector_store.insert -> db.batch_add_history(records)
      proxy: inner.batch_add_history(records)        # mem0 commit first
             for r in records: receipt(op=ADD, memory_id=r.memory_id,
                 content_hash=blake3(r.new_memory), engine_ts=ms(r.created_at), scope=_SCOPE)
             one store transaction, consecutive seq
  wrapper: _SCOPE.reset(); raise stored ReceiptError if any (D-8); return mem0 result

update(memory_id, data, metadata)
  wrapper: row = mem0.get(memory_id); scope from row's user_id/agent_id/run_id -> _SCOPE
  mem0._update_memory -> db.add_history(memory_id, old, new, "UPDATE", ...)
      proxy: receipt(op=UPDATE, content_hash=blake3(new), prev_content_hash=blake3(old),
             engine_actor=actor_id, engine_role=role, engine_ts=ms(updated_at))

delete(memory_id)
  wrapper: row = mem0.get(memory_id) -> _SCOPE        # payload is gone when add_history runs
  mem0._delete_memory -> db.add_history(memory_id, old, None, "DELETE", is_deleted=1)
      proxy: receipt(op=DELETE, prev_content_hash=blake3(old))

delete_all(user_id=.., ...)
  wrapper: scope from arguments -> _SCOPE; mem0 loops _delete_memory; one DELETE receipt each

reset()
  wrapper: receipt(op=RESET, scope={}) first, then mem0.reset(), then re-wrap the new obj.db (§6.5)
```

The pre-read in `update` and `delete` is stable: mem0 strips identity keys from update
metadata, so the scope of a memory cannot change (`main.py:2074`). If `get` returns `None`,
the wrapper calls mem0 unchanged and mem0 raises its own `ValueError`.

A mutation that reaches the proxy with no scope in the context (an internal call path this
spec does not know) is written to the empty scope and logged. With `strict_scope=True` the
wrapper raises a deferred `ReceiptError` instead. The receipt still exists in both cases, so
the ledger chain stays complete.

### 6.4 Async and threads

- `ContextVar` values follow the task. `asyncio.to_thread` copies the context, so the proxy
  sees the scope inside the worker thread (`main.py:2772`, `3695`).
- `LedgerStore` serialises emission with one `threading.Lock` and one SQLite transaction
  (`BEGIN IMMEDIATE`). Two threads that add at the same time get consecutive, never
  duplicate, `seq` values.
- The proxy is synchronous, like `SQLiteManager`. No `await` happens while the lock is held.

### 6.5 `reset()`

mem0 replaces `self.db` after `db.reset()` (`main.py:2153-2155`). The wrapper therefore:

1. Emits the `RESET` receipt before the call (the engine state after the call is empty).
2. Calls the original `reset`.
3. Wraps the new `obj.db` again. Without this step every later mutation would be silent.

### 6.6 Failure semantics and `audit`

- Inner write fails: the proxy re-raises. No receipt. mem0 handles its own fallback.
- Inner write succeeds, receipt write fails: the proxy stores the error and returns normally
  (D-8). The public wrapper raises `ReceiptError` after mem0 returns. The ledger then has a
  gap relative to the mem0 history table.
- Process dies between the inner write and the receipt: the same gap.

`ledger.audit(history_db_path)` reads the mem0 `history` table (`storage.py:108-120`) and
matches every row to a receipt by `(memory_id, op, blake3(new_memory), blake3(old_memory),
ms(updated_at or created_at))`. It reports: rows without a receipt (gaps), receipts without a
row (history pruned or edited), hash mismatches (history edited), and rows older than the
ledger (`pre-ledger`). The audit makes mem0's own log tamper-evident, without changing mem0.

### 6.7 Store schema (SQLite, `~/.mnemonik/ledger/<ledger_id>.db` by default)

```sql
CREATE TABLE meta        (key TEXT PRIMARY KEY, value TEXT NOT NULL);
  -- ledger_id, producer, engine, created_ms, schema = 'receipt.v1'
CREATE TABLE receipts    (seq INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
                          scope_id TEXT NOT NULL, scope_seq INTEGER NOT NULL,
                          op TEXT NOT NULL, memory_id TEXT, ts INTEGER NOT NULL,
                          cose BLOB NOT NULL);
CREATE INDEX receipts_scope ON receipts (scope_id, scope_seq);
CREATE INDEX receipts_memory ON receipts (memory_id);
CREATE TABLE scopes      (scope_id TEXT PRIMARY KEY, scope_json TEXT NOT NULL);  -- raw scope, local only
CREATE TABLE checkpoints (idx INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE,
                          from_seq INTEGER, to_seq INTEGER, root TEXT NOT NULL,
                          cose BLOB NOT NULL, anchor TEXT);
CREATE TABLE failures    (at_ms INTEGER NOT NULL, after_seq INTEGER, error TEXT NOT NULL);
CREATE TRIGGER receipts_ro_u BEFORE UPDATE ON receipts BEGIN SELECT RAISE(ABORT,'append-only'); END;
CREATE TRIGGER receipts_ro_d BEFORE DELETE ON receipts BEGIN SELECT RAISE(ABORT,'append-only'); END;
```

The triggers stop accidental edits. They do not stop the operator. Checkpoints and the
`audit` command are the tamper-evidence; the triggers are hygiene.

### 6.8 Export format (JSON Lines)

```
{"ledger_id":"6f1c…","producer":"7f3a…","engine":"mem0-oss/2.2.1","schema":"receipt.v1","exported_ms":1760180000000}
{"seq":0,"id":"<hex>","cose":"<base64>"}
{"seq":1,"id":"<hex>","cose":"<base64>","scope":{"user_id":"u1"}}      # only with --with-scopes
{"checkpoint":{"idx":0,"id":"<hex>","cose":"<base64>","anchor":"arweave:<tx>"}}
```

`export(scope=...)` writes only the receipts of that `scope_id`, so the per-scope chain
verifies on its own (D-3). `since_seq` gives incremental exports.

### 6.9 Verify algorithm (`mnemonik verify <file>`)

1. Read the header. Trusted keys = `--key` values, else the header `producer` with the warning
   `self-asserted key`.
2. For each receipt line: decode COSE; check `kid` is trusted; verify the Ed25519 signature;
   recompute `receipt_id = blake3(cose)` and compare with `id`; decode the payload by
   `RECEIPT_V1`; check `producer == kid` and `ledger_id` equals the header.
3. Ledger chain: `seq` rises by one; `prev` equals the previous `id`. A missing `seq` is a
   gap: an error with `--complete`, a warning otherwise.
4. Scope chain: per `scope_id`, `scope_seq` rises by one and `scope_prev` equals the previous
   `id` in that scope. When a `scope` object is present, recompute `scope_id` and compare.
5. Checkpoints: verify the signature; recompute the root over the exported ids in
   `[from_seq, to_seq]`; compare. Report receipts after the last checkpoint as `uncommitted`.
   `--online` fetches `anchor` locators and compares the bytes.
6. Report: receipts per op, chain status with the first break, unscoped count, keys seen,
   first and last `ts`, checkpoint coverage. Exit 0 only when every signature is valid and no
   chain is broken.

### 6.10 Keys

- `Signer.from_file(path)` reads `{secret: number[64], pubkey_base58}` (the CLI format, §2).
  Seed = `secret[:32]`. The derived public key must equal `pubkey_base58`.
- `Signer.generate(path)` writes the same format with mode `0600` in a `0700` directory.
- Lookup order: `signer=` argument, `MNEMONIK_IDENTITY` path, `~/.mnemonic/identity.json`
  (shared with the CLI), else generate.
- v1 uses a file key only. The OS keychain stores in `core/src/identity/` are not used.
- `Signer.from_seed(bytes)` exists for tests and fixtures.

### 6.11 Platform adapter (planned, after OSS)

`MemoryClient` has no local history writer. The adapter wraps the public methods and builds
receipts from what the client saw:

| Method | Receipt source |
|---|---|
| `add` | each item in the response `results` with an `id` and a text; scope from the arguments |
| `update` | `get(memory_id)` before the call for the old text; response for the new text |
| `delete` | `get(memory_id)` before the call; with `delete_linked=True`, one receipt per id the response lists |
| `delete_all` | `get_all` with the same filters before the call (paged); one `DELETE` receipt per memory |
| `batch_update`, `batch_delete` | one receipt per item, same rules |

`engine = "mem0-platform/<client version>"`. `engine_actor` is absent. Known limit: the
receipt records the response the client saw, not the platform's internal state.

### 6.12 Public API

```python
class Ledger:
    @classmethod
    def open(cls, path: str | None = None, *, signer: Signer | None = None,
             on_error: Literal["raise", "warn"] = "raise", strict_scope: bool = False) -> "Ledger": ...
    @classmethod
    def wrap(cls, memory, *, ledger: "Ledger | None" = None,
             allow_unverified_engine: bool = False, **open_kwargs): ...   # returns the same object
    def unwrap(self, memory) -> None: ...
    def export(self, path, *, scope: dict | None = None, since_seq: int = 0,
               with_scopes: bool = False) -> ExportSummary: ...
    def checkpoint(self) -> Checkpoint: ...
    def audit(self, history_db_path: str) -> AuditReport: ...
    def receipts(self, *, scope: dict | None = None, memory_id: str | None = None) -> Iterator[Receipt]: ...
    head: tuple[int, str | None]      # (seq, receipt_id) or (-1, None)
    producer: str

def verify(path, *, trusted_keys: list[str] | None = None, complete: bool = False) -> VerifyReport: ...
```

CLI: `mnemonik keygen`, `mnemonik verify <file> [--key K] [--complete] [--online]`,
`mnemonik export <ledger> <file> [--scope k=v] [--since N] [--with-scopes]`,
`mnemonik checkpoint <ledger>`, `mnemonik audit <ledger> --history <mem0 history.db>`.
Anchoring a checkpoint reuses the Rust CLI (`arweave` module) and stays a separate command.

## 7. Cross-language parity

1. Add `RECEIPT_V1` and `CHECKPOINT_V1` to `core/src/codec/schema.rs` with the field orders
   in §4 and §5, and `ArtifactType::Receipt`, `ArtifactType::Checkpoint`. Append only.
2. Extend `core/tests/golden_fixtures.rs` with receipt and checkpoint entries: genesis `ADD`,
   `UPDATE` with `prev`, `DELETE`, `RESET`, one checkpoint over four receipts. Fixed seed,
   fixed integer timestamps.
3. The Python test `tests/test_golden.py` reads the same JSON. It asserts that
   `canonical_cbor_hex` and `cose_envelope_hex` are byte-identical, and that `verify_cose`
   accepts every Rust envelope.
4. The reverse check: a Rust test verifies a Python-produced export committed under
   `core/tests/fixtures/receipts/`.
5. `scope_id` fixtures: three scope maps, including a value that parses as RFC 3339, to pin D-7.
6. Merkle parity: the Python `commitment_root` over the fixture ids must equal the Rust root.

The Python canonicaliser implements only what the receipt schemas need: text, integers,
omitted nulls, sorted nested keys. It rejects floats and RFC 3339 strings with an error
instead of converting them. That keeps the Python code small and the parity rule explicit.

## 8. Security considerations

- **What a receipt proves.** The holder of `producer` attested at `ts` that the engine reported
  this mutation with these hashes. It does not prove the text is true, or that the engine
  stored it durably, or the wall-clock time.
- **Operator omission or fork.** The operator can stop writing receipts or fork the ledger.
  `audit` detects gaps against the mem0 history, but both files are under the same operator.
  Only a checkpoint held by a third party (anchored, or sent to a notary) makes omission after
  that point visible to outsiders. The hosted notary is planned, not part of v1.
- **Time.** `ts` is self-asserted. An anchored checkpoint gives an upper bound: the receipts
  existed before the anchor time.
- **Guessable text.** `content_hash` of a short, known text can confirm a guess. v1 uses the
  plain hash because third-party verification needs it. A per-ledger salt mode
  (`content_hash = blake3(salt || text)`, salt exported on request) is planned (Q-4).
- **Key compromise.** No rotation in v1. A new key starts a new ledger. A `KEY_ROTATION` op
  that links two ledgers is planned.
- **`engine_hash`** is MD5 and informational. The verifier never relies on it.
- **No network in the write path** (D-11). The store and the key file use restrictive modes.

## 9. Testing

- Unit: canonical CBOR rules, `scope_id`, COSE sign and verify, key file read and create,
  schema validation (missing required field, wrong type, reordered input gives the same bytes).
- Golden: §7 items 3 to 6.
- Wrap (mem0 OSS), with mem0's own test style (`tests/test_memory.py:23`, mocked LLM,
  embedder and vector store):
  - `add(infer=False)` → one `ADD` receipt per message, scope from arguments.
  - `add(infer=True)` → batch path: N receipts in one transaction, consecutive `seq`.
  - batch fallback: make the inner `batch_add_history` raise; mem0 falls back to
    `add_history`; exactly N receipts, no duplicates.
  - deferred raise: inner batch succeeds, receipt store fails; `add` raises `ReceiptError`;
    the mem0 history table has exactly N rows (no duplicate fallback rows).
  - `update` with text, `update` with metadata only (`content_hash == prev_content_hash`).
  - `delete`, `delete_all` (loop, one receipt each), unknown id (mem0 error, no receipt).
  - `reset`: `RESET` receipt, then a mutation after reset still gives a receipt (re-wrap).
  - `AsyncMemory` variants of all of the above.
  - two threads adding concurrently: `seq` contiguous, no duplicate ids.
  - unscoped internal call with and without `strict_scope`.
- Export and verify: round trip; flip one byte → signature error; drop one line → gap;
  reorder two lines → chain break; per-scope export verifies alone; checkpoint root matches.
- Audit: delete one mem0 history row → "receipt without row"; edit a row's text → hash mismatch.

## 10. Tasks

| Task | Wave | Size | Summary |
|---|---|---|---|
| 1 | 1 | S | Rust: `RECEIPT_V1`, `CHECKPOINT_V1`, golden fixture entries (§7.1-2) |
| 2 | 1 | M | Python: `codec`, `cose`, `keys`, `schema`, golden parity test (§7.3-6) |
| 3 | 2 | L | Python: `store`, `ledger`, `engines/mem0_oss` (§6.2-6.7) |
| 4 | 2 | M | Python: export, `verify`, CLI `verify` and `export` (§6.8-6.9) |
| 5 | 3 | M | Checkpoints, Merkle parity, `checkpoint` command, anchor via Rust CLI (§5) |
| 6 | 3 | M | `audit` against the mem0 history table (§6.6) |
| 7 | 4 | M | Platform adapter (§6.11) |
| 8 | 4 | S | Docs (§11), mem0 `integrations/` folder and marketplace entry behind an accepted upstream issue |

Tasks 1 and 2 touch different repos and run in parallel. Task 3 depends on 1 and 2.

## 11. Docs to update with the code

- `docs/comparisons.md`: the memory-only table gets a "signed receipts" row and the composable
  note changes to "run Mnemonic receipts under mem0".
- `docs/source-capabilities.md`: new `python/mnemonik` row, marked available when it ships.
- `README.md` and `CLAUDE.md`: the `python/` workspace member and its commands.
- `docs/WHITEPAPER.md`: one paragraph on receipts as the audit layer for third-party engines.
