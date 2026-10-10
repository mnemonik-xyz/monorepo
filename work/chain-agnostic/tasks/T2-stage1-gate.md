---
status: pending
wave: 1
blocked_by: [live-data-verification]
---

# T2: Acceptance gate — previous-bundler superset of memo enumeration

Run the live parity check with the environment variables read by `core/examples/enumerate.rs`:
```bash
env -u BUNDLER_ONLY \
  WALLET='<known-production-wallet>' \
  BUNDLER_GRAPHQL_URL='https://{previous-bundler-host}/graphql' \
  SOLANA_RPC_URL='<configured-rpc-url>' \
  cargo run -p mnemonic-core --example enumerate
```

The previous bundler's output must be a superset of memo output for a wallet with known historical anchors.
Record revision, date, redacted endpoint configuration, source counts, missing IDs and process exit status.
An empty memo set or an bundler-only run does not establish this production gate.
Do not publish RPC credentials in evidence.

## Review — 2026-10-02

No passing live comparison was found in the reviewed records.
`1edebe7` already changed ordinary write paths despite this unclosed prerequisite.
The direct sealed route still submits memos. This task remains pending; mocked tests do not close it.
See [baseline audit](../../protocol-product/baseline-audit.md).
