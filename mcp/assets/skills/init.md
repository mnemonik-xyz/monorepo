# mnemonik-init

## Purpose

Guide the user through first-time setup so Mnemonic is wired into their MCP-capable agent host (Claude Code, Claude Desktop, Cursor) and the local pipeline is ready for offline attestations. Use when the user is connecting to Mnemonic for the first time, or when the agent has detected the CLI is not installed.

## Trigger

**Positive examples (DO use):**

- The user is connected to the hosted `mcp.mnemonik.xyz` endpoint anonymously and asks "how do I install this locally" or "how do I save things".
- A `tools/call` returned an error indicating the local binary is not present (e.g., user is on hosted-only and tries a local-mode write).
- The user explicitly asks for setup instructions, "how do I install mnemonik", "get started with mnemonik".
- `mnemonik-status` reports the host config does not contain a mnemonik entry.

**Negative examples (DO NOT use):**

- The user already has a working install (e.g., `mnemonik-status` says all checks pass).
- The user is mid-operation (writing an attestation, doing a recall) — do not detour through install.
- The user is on a hosted-only path and explicitly does not want to install anything locally — respect that and offer anonymous recall instead.

## Context to gather

- Platform: macOS, Linux, Windows. v1 only supports macOS for the install command; Linux/Windows are v1.1+.
- Whether the user has npm available. The shim ships through npm (`@mnemonik-xyz/mcp`).
- Which MCP hosts the user uses (Claude Code, Claude Desktop, Cursor). The install command writes to each detected host config.

## Tool

No direct MCP tool call. This skill produces a setup recipe:

```
npm install -g @mnemonik-xyz/mcp
mnemonik-mcp install
```

Then instruct the user to restart any running MCP host. After restart, the host will spawn `mnemonik-mcp mcp-stdio` as a subprocess and tools become available with full descriptions.

## Guardrails

- Do not offer to run the `npm install` command for the user via shell — it requires global permissions and may need `sudo` depending on their npm prefix.
- Do not modify host config files yourself; that is what `mnemonik-mcp install` does, with idempotency and non-destructive merge guarantees.
- For platforms outside the supported v1 list (Linux, Windows, Cline, Codex, Windsurf), say plainly that v1.1+ adds support and offer the anonymous-discovery hosted path as the meantime.
- Do not promise OAuth-free participate writes. Local mode is OAuth-free; participate (chain-anchored) requires the OAuth-loopback flow on first use.
