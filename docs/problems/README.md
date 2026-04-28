# Open Problems

This section holds open system problem statements and pricing validation that
the Mnemonic Protocol roadmap must address before broad multi-party deployment.
Each document captures a known gap between the current single-owner Rust MCP
implementation and the multi-agent, economically-validated production target.

## Documents

- [`MEMORY_EVICTION.md`](./MEMORY_EVICTION.md) — lifecycle, retention, and
  pruning policy as an open problem. What gets kept, what gets demoted, and
  how that interacts with permanent Arweave storage.
- [`CONCURRENT_WRITERS.md`](./CONCURRENT_WRITERS.md) — multi-agent shared-context
  write semantics as an open problem. Covers DAG hash chains, fork resolution,
  encryption for multi-party access, and Solana/Arweave constraints under
  concurrent commit pressure.
- [`ARWEAVE_PRICING_VALIDATION.md`](./ARWEAVE_PRICING_VALIDATION.md) —
  economic-model validation for full-mode persistence costs, sanity-checking
  the per-memory and per-namespace cost envelope against current Arweave
  pricing.

## Status

These documents informed the follow-up roadmap captured in
[`../../work/docs-actualization/decisions.md`](../../work/docs-actualization/decisions.md);
they remain open problems pending dedicated design and validation work.
