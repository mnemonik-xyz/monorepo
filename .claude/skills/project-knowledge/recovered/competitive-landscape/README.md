# Competitive landscape

Recovered competitive-landscape and trustless-RAG research. All four documents were originally written against an earlier prototype state of Mnemonic; the Mnemonic-specific claims have been validated against `mnemonik-xyz/monorepo@main` and either edited inline or wrapped in a clear validation note.

## Files

- **DECENTRALIZED_RAG_LANDSCAPE.md** — broad academic survey of decentralized / trustless RAG protocols, ZK retrieval, content-addressed storage, and verifiable ANN search. Evergreen — no Mnemonic-specific claims.
- **DRAG_ANALYSIS.md** — analysis of Lu et al. (arXiv:2511.07577) "D-RAG" against Mnemonic. **Validated:** SHA3 references replaced with `blake3 over canonical CBOR + COSE_Sign1`; AES-256-GCM "by default" reframed as roadmap; ADR references reframed as design lineage; capability table now reflects shipped `core/src/{codec,compress,identity,storage,solana,arweave,lineage}/` modules.
- **WEB_RESEARCH_TRUSTLESS_RAG.md** — survey of zkTAM, V3DB, JOLT Atlas, ERC-8004, and the Arweave + Solana production stack. **Validated:** zkTAM-vs-Mnemonic table updated; status "Pre-V1, prototype validated" → "active Rust MCP server (5 tools, HTTP + stdio, `local` / `full` modes)"; encryption framing corrected.
- **MCP_SERVER_BACKEND_FEATURES_COMPARISON.md** — historical Python (`mcp-server/`) vs Rust (`mcp-server-rs/`) backend comparison from `sivo4kin/mnemonic-protocol@feat/auth`. **Archived:** the Python backend has been retired. Document carries a prominent archive header and links to the old branch (`feat/auth`, `legacy`) where the Python code still lives, and to the current Rust backend at `mnemonik-xyz/monorepo@main:mcp/`.

## Reading order

If you want competitive positioning quickly:

1. `WEB_RESEARCH_TRUSTLESS_RAG.md` § 3 (positioning table) and § 4 (implications).
2. `DRAG_ANALYSIS.md` § 6 (strategic implications).
3. `DECENTRALIZED_RAG_LANDSCAPE.md` for the full academic context.

`MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` is only relevant if you are spelunking through Mnemonic's history (why the codebase consolidated on Rust) — it should not be used as a guide to the current backend.
