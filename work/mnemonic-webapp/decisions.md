# Decisions Log: mnemonic-webapp

## Task 1: Extend MCP config with Ollama env vars

Added `OLLAMA_URL`, `OLLAMA_MODEL`, and `RAG_CHUNK_DIR` to `config.rs` with corresponding `McpState` fields in `mcp.rs`. OLLAMA_URL is validated at startup using the `url` crate against a strict hostname whitelist (only `localhost` and `ollama` allowed, IP addresses rejected) per Decision 8 (SSRF prevention). Nine unit tests cover the validation logic including edge cases like loopback IP bypass attempts.

## Task 2: RAG seeding -- whitepaper chunking + sign_memory + artifact generation

Created `mcp/src/seed.rs` implementing the startup seeding routine: parses `docs/WHITEPAPER.md` at `## ` headers (with h3 sub-splitting for sections exceeding ~500 tokens), calls `sign_memory()` per chunk with tags `["protocol-knowledge", "whitepaper"]`, and generates a `.zip` artifact containing `knowledge.md` (YAML frontmatter with content_hash/signer_pubkey/timestamp per chunk) and `knowledge.json` sidecar. Added `zip` crate to `mcp/Cargo.toml`. Seeder is called from `main.rs` after McpState init; skips if `store.count() > 0` (idempotent). The canonical `.zip` path is stored in a new `McpState.artifact_zip_path` field for the future `/download-knowledge` handler. Key fix during review: the h2 parser incorrectly matched `### ` lines (since they start with `## `) -- fixed with explicit exclusion and regression test. 13 unit tests cover parsing, splitting, and artifact generation.
