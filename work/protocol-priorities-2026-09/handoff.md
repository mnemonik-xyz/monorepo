---
created: 2026-09-27
status: in_progress
type: handoff
---

# Handoff: issue triage and protocol priorities (2026-09-27)

This file records the state of the work so that another session can continue it.

## Owner priorities

1. Finish paid on-chain anchoring (x402, #204).
2. Deploy the new version. Production runs `a7cde35` (last `deploy-mcp.yml` run on 2026-07-13).
3. Only the agent signs anchored memories. The MCP server must not sign them.
4. Give each agent key a free daily quota of anchored writes (10 per UTC day, plus a global daily cap).
5. Make the MCP easy for agents. The OS keychain prompts only for anchored (participate) writes.
6. Register and renew the server in public MCP registries.
7. Test every change. Do not break things.
8. Update the docs in the same change as the code.

## Done

- Issue triage: 34 issues closed with comments. The remaining issues carry priority labels.
  - P1 by owner decision: A2A (#61–#66, new umbrella #233), validator and reputation oracle (#72, #73), ERC-8004 (#69, #70).
  - Deferred label: #71, #74, #75.
- PR #234 (merged as `db0ab28`): MCP registry publishing.
  - `server.json` (passes `mcp-publisher validate`), `mcpName` in `packages/mcp/package.json`.
  - `publish-mcp-registry` job in `release.yml`, updated `smithery.yaml`, guide `docs/mcp-registries.md`.

## Open PRs (state on 2026-09-27)

All branch from `main` at `d4deb60`, except #238 (from `db0ab28`).

| Branch / PR | Scope | State | Next step |
|---|---|---|---|
| `claude/hopeful-newton-9x6nwt-cli` (#237, ready) | CLI/SDK 0.3.0: lazy signer, `mnemonic sign` defaults to `local`, `--anchor` for participate, refresh-token session renewal (#33) | Done. SDK 172/172, CLI 157 pass (+2 skipped); `tsc` clean. | Merge after #235 is deployed, then publish CLI 0.3.0. |
| `claude/hopeful-newton-9x6nwt-signing` (#235, draft) | Server signing rules (below), `tools::Transport` guard, `payment::active_universal_paywall` (UP charge only with `PAYMENT_MODE=x402`), docs incl. CLAUDE.md | Code complete (`b2f051d`, `2610f08`). Agent run: mcp 683 pass, core 222 pass, fmt and clippy clean. Independent re-run: fmt and clippy clean; full test suite not finished. | Run the full test gate. Update the PR body. Mark ready. |
| `claude/hopeful-newton-9x6nwt-bugfixes` (#236, draft) | #165 whoami pricing, #201 recall with chain rows, #163 OAuth 401 challenge | #165 committed (`e40222b`). #201 committed (`23fcd05`). #163 WIP snapshot (`d90e530`, untested: `oauth/mod.rs`, `auth_allowlist.rs`, `discovery_anonymous.rs`). | Finish #163 (see below), run the gate, update the PR body, mark ready. |
| `claude/hopeful-newton-9x6nwt` (#238, ready) | Docs: docs-with-code rule in CLAUDE.md, this hand-off | Docs only. | Merge. |

### #163 target behaviour

- Auth-required tool calls with a missing, invalid or expired token return HTTP 401 and `WWW-Authenticate: Bearer resource_metadata="https://mcp.mnemonik.xyz/.well-known/oauth-protected-resource/mcp"` (add `error="invalid_token"` for a bad token).
- Source: https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization
- Anonymous `initialize`, `tools/list` and anonymous `mnemonic_recall` keep working. An expired token on `initialize` gets 401, so the client refreshes.
- On 2026-09-27 production returned HTTP 200 to an unauthenticated `tools/call`. Smithery detects OAuth only from a 401.

### Server signing rules (target behaviour)

- Hosted HTTP, JWT caller, `mode: "local"`: the server stores a hash-only row owned by the JWT subject. Nothing is signed. No client keychain access.
- Hosted HTTP, JWT caller, `mode: "participate"`: deferred path. The client signs the COSE bundle; `/api/sign-callback` verifies `kid == JWT sub` and anchors the client bytes unchanged.
- HTTP without JWT: operator signing of a memory is impossible.
- stdio: the local agent's own identity (lazy keychain) signs participate writes. That key belongs to the agent, not to a hosted operator.
- The server wallet signs only transport records as fee payer: the Arweave DataItem and the Solana memo transaction. Verifiers use the COSE `kid`.

## Not started

- Free anchoring quota (no code yet; branch `claude/hopeful-newton-9x6nwt-quota` does not exist on the remote). Build it on top of `claude/hopeful-newton-9x6nwt-signing`. It reuses `payment::active_universal_paywall` and `tools::Transport`. Design:
  - `MNEMONIC_FREE_ANCHORS_PER_DAY` (default 10) per Ed25519 key, plus `MNEMONIC_FREE_ANCHORS_GLOBAL_PER_DAY`.
  - Logic in `mcp/src/payment.rs` (`try_consume_free_anchor`, `refund_free_anchor`); an mcp-owned migration for table `free_anchor_usage(subject, day, n)`.
  - Call sites: `api.rs` sign-callback before the paywall gate (key = verified `signer_pubkey`), and the pre-parking gate in `mcp.rs`.
  - Refund on demotion. Report `free_anchors_remaining` in whoami and in the 402 body.
- Version bump to 0.3.0 (`core/Cargo.toml`, `mcp/Cargo.toml`, `Cargo.lock`, `packages/mcp`). Then tag a release and run `deploy-mcp.yml` (apply, then smoke). The owner runs the deploy.
- x402 conformance tasks M1–M3 (`work/x402-v2-conformance/tasks/`).
- Privacy review of anchored memories: private vs public, what goes on Arweave, encryption, private sharing with a specific agent. Not finished; run it again. Known so far: no code shares a memory privately with a specific agent. The design is in `docs/spec/memory-composition.md` and issue #211.
- Follow-ups from the signing change:
  - The `-32011` refund and quota logic in the x402 branch of `mcp_handler` can no longer fire over HTTP. It is dead code; remove it in a later PR.
  - SDK `verify` on a hash-only local row: check the server response.

## Test gate for every PR

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features mnemonic-mcp/test-support -- -D warnings
cargo test --workspace --no-fail-fast --features mnemonic-mcp/test-support
npm test -w @mnemonik-xyz/sdk && npm test -w @mnemonik-xyz/cli
```

Also run a local HTTP smoke test (`STORAGE_MODE=local PAYMENT_MODE=none`): `whoami`, `tools/list`, a local write, `recall`.

## Environment notes

- Do not share one `CARGO_TARGET_DIR` across git worktrees. Cargo can link a workspace crate built from another worktree and fail with bogus errors (seen: E0063). If you must share it, touch `core/` and `mcp/` sources before each gate run.
- A debug `target/` for the full test suite needs about 18 GB. On a small disk, use `CARGO_INCREMENTAL=0` and share one `CARGO_TARGET_DIR` across worktrees.
- CI runs nightly and on manual dispatch only. Nothing gates a PR, so run the gate above before each push.
