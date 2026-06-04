# @mnemonik-xyz/mcp

MCP shim for the Mnemonic Protocol. Ships `mnemonik-mcp` — a thin Node entrypoint
that lazily downloads and verifies the matching `mnemonic-mcp` Rust binary from
GitHub Releases on first invocation, then dispatches subcommands to it.

No `postinstall` script. The first invocation performs the download +
SHA256 + `gh attestation verify` ceremony and caches the binary under
`~/.local/share/@mnemonik-xyz/mcp/bin/mnemonik-mcp`.

## Install

```bash
npm install -g @mnemonik-xyz/mcp
```

Requires Node 20 or newer; the `gh` CLI is required for signature
verification on first run.

## Subcommands

- `mnemonik-mcp install` — write `mcpServers.mnemonik` into any present host
  config (`~/.claude.json`, `~/Library/Application Support/Claude/claude_desktop_config.json`,
  `~/.cursor/mcp.json`). Non-destructive (merge), idempotent, atomic write.
- `mnemonik-mcp install --check` — dry run; prints the plan without touching files.
- `mnemonik-mcp install --confirm` — interactive prompt before each write.
- `mnemonik-mcp mcp-stdio` — spawn the cached binary's `mcp-stdio` subcommand
  with stdio inherited (this is what host configs point at).
- `mnemonik-mcp doctor` — run six diagnostics and exit 0/1.

See `work/agent-native-distribution/tech-spec.md` for design notes.
