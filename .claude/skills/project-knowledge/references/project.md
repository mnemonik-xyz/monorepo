# Project Context

## Purpose
This file provides high-level project overview for AI agents. Helps agents understand WHAT we're building and WHY.

---

## Project Overview

**Name:** mnemonic-protocol (monorepo)

**Description:** Monorepo for the Mnemonic Protocol — verifiable, persistent memory for AI agents with cryptographic attestation on Arweave and Solana.

Contains four components: protocol documentation, a Rust core library, an MCP server for Cursor/Claude Desktop, and a demo web app. The core library compiles to both native Rust and WASM, so all components share one implementation. See architecture.md for the full directory structure.

---

## Target Audience

**Primary users:** Developers building AI agents who need persistent, tamper-proof memory across sessions and provider switches.

**Use case:** Connect an AI agent to Mnemonic via MCP (Cursor, Claude Desktop) so it remembers decisions and context across sessions — and can cryptographically prove that memory has not been altered. Developers can also import `mnemonic-core` directly or use the WASM package in TypeScript.

---

## Core Problem

AI agents lose all context when a session ends. Even when memory is stored, there is no way to verify it has not been changed. Mnemonic solves both: it persists agent memory across sessions and anchors a cryptographic proof on-chain (Arweave for storage, Solana for timestamping), so any party can independently verify that a memory item is authentic and unmodified.

Provider portability is the secondary problem: memory stored via Mnemonic is provider-agnostic — a user can switch from Claude to GPT to a local model and continue working with the same attested memory.

---

## Key Features

- **Semantic embedding + TurboQuant compression** — encodes content as a dense vector, scalar-quantises to 2–4 bits per dimension (up to 32× compression), making on-chain storage practical
- **Cryptographic attestation** — blake3 hash over canonical CBOR bytes signed as a COSE_Sign1 artifact (legacy SHA-256/JSON artifacts still verifiable via fallback path); anchor memo on Solana, full signed bytes stored on Arweave/Irys via signed ANS-104 bundle item
- **Ed25519 identity** — every agent has a deterministic keypair; DID-sol and DID-key derivation built in
- **MCP server** — 5 tools (whoami, sign_memory, verify, prove_identity, recall) for Cursor and Claude Desktop; supports local mode (free, SQLite-only) and full mode (on-chain)
- **Demo web app** — project landing page + live demo: chat with local Qwen2.5-7B about the protocol, export attested context to any MCP client

---

## MVP Scope

MVP consists of `core/` and `mcp/`. Core ships as both a native Rust crate and a WASM npm package. MCP server is fully functional in local mode for Cursor and Claude Desktop.

`webapp/` and full `docs/` are prepared in parallel but not blocking MVP.

Post-launch ideas: Python bindings via pyo3, Go bindings, hosted attestation API.

---

## Out of Scope

- No Python or Go implementations in this repo
- No multi-tenant or shared-key scenarios in MVP
- No mobile SDK
- No collaborative memory pools
