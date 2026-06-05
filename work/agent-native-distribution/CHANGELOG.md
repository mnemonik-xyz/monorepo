# agent-native-distribution — Release Notes

## v0.2.3 (2026-06-05)

First coordinated release of the agent-native-distribution feature.

### Highlights

- **`@mnemonik-xyz/mcp` shim (new package)** — npm-installable MCP shim that lazily downloads + verifies the matching `mnemonik-mcp` Rust binary from GitHub Releases (sigstore-attested) on first invocation. Subcommands: `install`, `install --check`, `mcp-stdio`, `doctor` (6 health checks).
- **`mnemonic-mcp` binary v0.2.3** — `initialize`/`prompts/list`/`resources/list` endpoints, 7 enriched MCP tools (skill manifests with `Purpose:` + `Trigger:`), per-tool visibility filter (private vs public), opt-in `participate` write-mode confirmation gate.
- **`mnemonic-core` lib v0.2.3** — public `Visibility` enum added; minor semver bump.
- **CI** — release.yml now publishes SHA256SUMS + sigstore attestation for tarballs, and publishes `@mnemonik-xyz/mcp` via npm Trusted Publishing.

### Coordinated packages

| Package | Old | New |
|---|---|---|
| `mnemonic-mcp` (Rust binary) | 0.1.0 | 0.2.3 |
| `mnemonic-core` (Rust lib) | 0.1.0 | 0.2.3 |
| `@mnemonik-xyz/mcp` (npm shim) | 0.2.0 | 0.2.3 |
| `@mnemonik-xyz/sdk` | 0.2.1 | unchanged |
| `@mnemonik-xyz/cli` | 0.2.2 | unchanged |

### Deferred to v1.1+

- AC11 (OAuth-loopback browser flow E2E) — covered in T14 post-deploy verification.
- AC15 (stderr mismatch warning) — binary-embedded discovery makes mismatch unobservable for v1 (per T11 audit consensus).
- Linux release artifacts — libdbus build issues; v0.2.3 ships macOS-only.
