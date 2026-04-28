# Execution Plan — mnemonic-cli

## Overview

15 tasks, 5 logical waves. Effective parallelism is governed by `depends_on` (some "waves" are gated by individual deps, not just wave numbers).

Branch: `feat/mnemonic-cli-userspec` (continues from approved tech-spec).

## Dependency-resolved order

```
Wave 1 (foundation, single task)
└─ T1: workspace + wasm-pack target investigation [no deps]

Wave 2 (after T1)
├─ T2: SDK core + Signer + LocalSigner + Keypair + contract suite
├─ T3: SDK OAuth (PKCE primitives, headless mode)
└─ T6: server-side OAuth allowlist + bootstrap-ticket endpoints + PKCE state binding [no deps]

Wave 3 (mixed deps)
├─ T4: COSE wrapper + golden fixture + CI lockstep gate [needs T2]
├─ T5: CLI commands [needs T2 + T3]
└─ T7: webapp IdentityPanel "Send to CLI" button [needs T6]

Wave 4 (after T4, T5, T6)
└─ T8: integration tests (mock server with fault injection) + cross-runtime CI matrix

Wave 5 (after T5)
└─ T9: documentation

Wave 6 (audit, parallel, reviewers: none) — after T8 + T9
├─ T10: code audit
├─ T11: security audit
└─ T12: test audit

Wave 7 (final) — after audit clean
├─ T13: pre-deploy QA
├─ T14: deploy (npm publish + server + webapp)
└─ T15: post-deploy verification
```

## Per-task spawning plan

| Task | Teammate | Reviewers | Verify | Files touched |
|---|---|---|---|---|
| T1 | T1-impl (general, opus, code-writing) | code-reviewer, security-auditor | smoke | root package.json, packages/sdk skeleton, packages/cli skeleton, build-wasm scripts |
| T2 | T2-impl | code-reviewer, security-auditor, test-reviewer | — | packages/sdk/src/{client,signer,keypair,cose,errors,types,index}.ts |
| T3 | T3-impl | code-reviewer, security-auditor, test-reviewer | — | packages/sdk/src/oauth.ts |
| T4 | T4-impl | code-reviewer, security-auditor, test-reviewer | smoke | core/Cargo.toml, core/tests/golden_fixtures.rs, packages/sdk/test/fixtures/ |
| T5 | T5-impl | code-reviewer, security-auditor, test-reviewer | smoke + user | packages/cli/{bin,src} |
| T6 | T6-impl | code-reviewer, security-auditor, test-reviewer | smoke | mcp/src/{oauth,api,main}.rs |
| T7 | T7-impl | code-reviewer, security-auditor, test-reviewer | smoke + user | webapp/src/components/IdentityPanel.tsx, webapp/e2e/cli-bootstrap.spec.ts |
| T8 | T8-impl | code-reviewer, test-reviewer, deploy-reviewer | smoke | packages/sdk/test/integration/, packages/cli/test/integration/, .github/workflows/node-test.yml |
| T9 | T9-impl | documentation-reviewer | smoke | packages/{sdk,cli}/README.md, SMOKE.md, JSDoc |
| T10 | T10-audit | none (audit IS the review) | — | reads all SDK + CLI + diff |
| T11 | T11-audit | none | — | reads all + security focus |
| T12 | T12-audit | none | — | reads tests + coverage |
| T13 | T13-qa | none | user | runs full test matrix |
| T14 | T14-deploy | none | smoke | npm publish + ssh ops |
| T15 | T15-postdeploy | none | user | fresh-install verification |

## User checks (final gate)

After Task 15 completes:
- `npm install -g @mnemonik-xyz/cli` on a fresh machine works.
- Bootstrap-ticket round-trip: webapp "Send to CLI" → CLI `identity import --ticket` → CLI sees same identity as Claude.ai.
- `mnemonic sign` → cross-tool recall in Claude.ai succeeds.
- Negative paths: arbitrary `redirect_uri` → 400; double-redeem → 410.

## Risks

- TeamCreate orchestration tooling (`~/.claude/teams/`, SendMessage between sub-agents) is not available in this environment. Lead spawns each teammate independently via the Agent tool; reviewers run sequentially after the teammate reports completion (lead orchestrates the review handoff manually instead of inter-agent messaging).
- Server changes in T6 require explicit user authorization for `systemctl restart mnemonic-mcp` during T14 (harness rule) — lead pauses + asks at that point.
- Volume: ~15 teammates + reviewers × ≤3 review rounds = up to 45–60 agent invocations. Each Wave 1+2 round is parallelizable; sequential commits per task to avoid git conflicts.

## Resume / checkpoint

`logs/checkpoint.yml` initialized with `total_waves: 7, last_completed_wave: 0`. After each wave, lead updates `last_completed_wave` and per-task status. Resume after context compaction: read checkpoint, skip completed tasks, recreate ad-hoc agents.
