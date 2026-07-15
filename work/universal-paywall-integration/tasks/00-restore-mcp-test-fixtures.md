---
status: complete
priority: P1
completed: 2026-07-15
---

# Restore MCP test fixtures after approval-state additions

## Outcome

Update every `McpState` test builder with the Universal Paywall and approval
fields required by the production struct.

## Validation

- `cargo test -p mnemonic-mcp universal_paywall --lib` compiles.
- `cargo test -p mnemonic-mcp chat --lib` passes.

## Delivered

Commit `1c8c951` updates the `mcp.rs` and `chat.rs` test builders.

