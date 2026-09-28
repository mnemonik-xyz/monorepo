---
status: pending
wave: 2
blocked_by: [T2]
---

# T3: Stage 2 — stop writing the memo

After gate passes:
- WriteMode::Anchored uploads to Arweave, does not call Solana
- solana_tx stays in schema, new rows store empty string
- read_memo, list_memo_anchors, parse_anchor_memo stay (legacy rows)
- Delivery check drops Solana comparison (Arweave re-fetch + COSE verify is sufficient)
