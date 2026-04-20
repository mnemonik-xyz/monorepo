# Project: mnemonic-protocol

> **Root repo for the Mnemonic Protocol — verifiable, persistent memory for AI agents with cryptographic attestation on Arweave and Solana.**

---

## How This Project Works

**Context:** All project knowledge is in `.claude/skills/project-knowledge/` with guides for architecture, patterns, and deployment.

**Default branch:** `dev`

**Library Documentation:** Always use context7 when you need code generation, setup or configuration steps, or library/API documentation. This means you should automatically use the Context7 MCP tools to resolve library id and get library docs without user having to explicitly ask.

## Structure

This is a **git submodule container**. Each subdirectory is an independent repo:

| Directory | Repo | Contents |
|---|---|---|
| `docs/` | (root repo) | Protocol specification, whitepaper, diagrams, ADRs, roadmap |
| `core/` | `mnemonic-core` | Rust library — native + WASM (`@mnemonic/core` npm package) |
| `mcp/` | `mnemonic-mcp` | MCP server binary for Cursor and Claude Desktop |
| `webapp/` | `mnemonic-webapp` | Demo web app + project landing page |

Clone: `git clone --recurse-submodules <url>`
Update submodules: `git submodule update --remote`

## Tech

- **Language:** Rust (`core/`, `mcp/`), TypeScript/React (`webapp/`)
- **Core targets:** native binary + `wasm32-unknown-unknown`
- **Local model (webapp):** Ollama + Qwen2.5-7B-Instruct
