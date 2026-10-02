---
status: in_progress
wave: 2
blocked_by: [T2]
---

# T3: Stage 2 — stop writing the memo

After gate passes:
- WriteMode::Anchored uploads to Arweave, does not call Solana
- solana_tx stays in schema, new rows store empty string
- read_memo, list_memo_anchors, parse_anchor_memo stay (legacy rows)
- Delivery check drops Solana comparison (Arweave re-fetch + COSE verify is sufficient)

## Review — 2026-10-02

Implementation `1edebe7` changed ordinary memory helpers and the sign callback.
`mcp/tests/stage2_no_solana_memo.rs` covers those paths with mocked sources.
The direct `anchor_sealed_handler` still submits a memo; product task 2 owns its shared ingestion migration.
T2 live parity remains pending. This status records partial implementation, not gate acceptance or production readiness.
