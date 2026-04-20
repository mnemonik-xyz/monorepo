# Project: mnemonic-protocol

> **Monorepo for the Mnemonic Protocol — verifiable, persistent memory for AI agents with cryptographic attestation on Arweave and Solana.**

---

## How This Project Works

**Context:** All project knowledge is in `.claude/skills/project-knowledge/` with guides for architecture, patterns, and deployment.

**Default branch:** `dev`

**Library Documentation:** Always use context7 when you need code generation, setup or configuration steps, or library/API documentation. This means you should automatically use the Context7 MCP tools to resolve library id and get library docs without user having to explicitly ask.

## Monorepo Structure

| Directory | Contents |
|---|---|
| `docs/` | Protocol specification, whitepaper, architecture diagrams, ADRs, roadmap |
| `core/` | Rust library crate — dual-target: native + WASM (`@mnemonic/core` npm package) |
| `mcp/` | MCP server binary for Cursor and Claude Desktop |
| `webapp/` | Demo web app + project landing page (TypeScript + React + WASM from core) |

## Tech

- **Language:** Rust (core + mcp), TypeScript/React (webapp)
- **Core targets:** native binary + `wasm32-unknown-unknown`
- **Local model (webapp):** Ollama + Qwen2.5-7B-Instruct
- **Cargo workspace:** root `Cargo.toml` with members `core` and `mcp`
