# Competitive landscape

Competitive-landscape and trustless-RAG research validated against the current Mnemonic Protocol implementation in `mnemonik-xyz/monorepo@main`. Mnemonic-specific claims have been edited inline against shipped behaviour (`blake3` over canonical CBOR + COSE_Sign1 anchored on Arweave + Solana, active Rust MCP server with 5 tools).

## Files

- **DECENTRALIZED_RAG_LANDSCAPE.md** — broad academic survey of decentralized / trustless RAG protocols, ZK retrieval, content-addressed storage, and verifiable ANN search. Evergreen — no Mnemonic-specific claims.
- **DRAG_ANALYSIS.md** — analysis of Lu et al. (arXiv:2511.07577) "D-RAG" against Mnemonic. Capability table reflects shipped `core/src/{codec,compress,identity,storage,solana,arweave,lineage}/` modules.
- **WEB_RESEARCH_TRUSTLESS_RAG.md** — survey of zkTAM, V3DB, JOLT Atlas, ERC-8004, and the Arweave + Solana production stack with positioning tables.

## Reading order

If you want competitive positioning quickly:

1. `WEB_RESEARCH_TRUSTLESS_RAG.md` § 3 (positioning table) and § 4 (implications).
2. `DRAG_ANALYSIS.md` § 6 (strategic implications).
3. `DECENTRALIZED_RAG_LANDSCAPE.md` for the full academic context.
