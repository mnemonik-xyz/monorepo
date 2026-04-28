---
created: 2026-04-26
feature: mnemonic-integrations
total_waves: 5
total_tasks: 15
status: pending_approval
---

# Execution Plan: mnemonic-integrations Phase 1 (Hackathon MVP)

## Scope

15 tasks across 5 waves, ~13 dev-days of work compressed via parallel execution. Output: hosted MCP at `mcp.mnemonik.xyz` + webapp at `mnemonik.xyz` + Smithery listing + browser-mediated signing flow + 12 integration tests in CI.

## Wave Plan

### Wave 1 — Foundation (3 tasks, fully parallel)

| Task | Name | Skill | Reviewers | Verify |
|---|---|---|---|---|
| T1 | Streamable HTTP transport upgrade | code-writing | security-auditor, test-reviewer | smoke |
| T2 | WASM bindgen wrappers in core | code-writing | security-auditor, test-reviewer | smoke |
| T3 | Webapp WASM build pipeline | infrastructure-setup | security-auditor, test-reviewer | smoke |

No shared files. Run all 3 in parallel.

### Wave 2 — Auth + Smithery (3 tasks, partial parallelism)

| Task | Name | Depends on | Skill | Reviewers | Verify |
|---|---|---|---|---|---|
| T4 | OAuth 2.1 + PKCE server module | T1 | code-writing | security-auditor, test-reviewer | smoke |
| T5 | Browser-mediated signing infrastructure | T2, T4 | code-writing | security-auditor, test-reviewer | smoke |
| T6 | Smithery listing + DNS subdomain + nginx | — | infrastructure-setup | security-auditor, test-reviewer | smoke, user |

**Sequencing within Wave 2:**
- Phase 2a: T4 + T6 in parallel
- Phase 2b: T5 after T4 lands

### Wave 3 — UI + Tests (3 tasks, partial parallelism)

| Task | Name | Depends on | Skill | Reviewers | Verify |
|---|---|---|---|---|---|
| T7 | Webapp landing + install + sign pages | T3, T4, T5 | code-writing | security-auditor, test-reviewer | smoke, user |
| T8 | 12 integration tests + MCP Inspector CI | T4, T5 | code-writing | security-auditor, test-reviewer | smoke |
| T9 | Pre-demo manual smoke checklist | T7, T8 | documentation-writing | none | user |

**Sequencing within Wave 3:**
- Phase 3a: T7 + T8 in parallel
- Phase 3b: T9 after T7+T8 land

### Wave 4 — Audit (3 tasks, fully parallel, read-only)

| Task | Name | Depends on | Skill | Reviewers |
|---|---|---|---|---|
| T10 | Code Audit | 1-9 | code-reviewing | none (auditor IS review) |
| T11 | Security Audit | 1-9 | security-auditor | none (auditor IS review) |
| T12 | Test Audit | 1-9 | test-master | none (auditor IS review) |

All write findings to `decisions.md`. If any flag critical issues → spawn ad-hoc fixer teammate with the auditors as reviewers (max 3 rounds).

### Wave 5 — Final (3 tasks, strictly sequential)

| Task | Name | Depends on | Skill | Reviewers |
|---|---|---|---|---|
| T13 | Pre-deploy QA | 3, 7, 9, 10, 11, 12 | pre-deploy-qa | none (QA is its own review) |
| T14 | Deploy hosted MCP + webapp | T13 | deploy-pipeline | security-auditor, test-reviewer |
| T15 | Post-deploy QA on live mc.mnemonik.xyz | T14 | post-deploy-qa | none |

## High-risk Actions Requiring Explicit User Confirmation

**Per system safety rules**, these tasks modify shared / production infrastructure and need an explicit "go" from the user before the corresponding teammate executes:

1. **T6 — DNS + nginx + certbot on VPS** (`150.251.147.215`)
   - DNS A-record creation for `mcp.mnemonik.xyz` (likely needs domain-registrar console access — user-only)
   - SSH to VPS, edit nginx config, run certbot to issue SSL cert
   - **User must confirm**: DNS update is approved + provide SSH access if not configured

2. **T6 — Smithery submission**
   - Submit MCP server listing to smithery.ai (requires user account + manual web form)
   - **User must perform** the actual submission; teammate prepares the smithery.yaml only

3. **T14 — Deploy to VPS**
   - SSH `claude@150.251.147.215`, git pull, cargo build --release, restart systemd service, deploy webapp
   - **User must confirm**: VPS access available + production deploy approved

## Operational Risks

- **Multi-submodule changes**: tasks span 3 git submodules (`core/`, `mcp/`, `webapp/`). Each submodule has its own git history. Commits per teammate must respect submodule boundaries.
- **fastembed model download**: T13 (Pre-deploy QA) runs `EMBED_PROVIDER=fastembed` which downloads ~22MB ONNX model on first run. CI pipeline / local dev environments need this cached.
- **wasm-pack build-time dep**: T3 requires `cargo install wasm-pack` in CI / dev setup. Document in deployment.md as prerequisite.
- **Time budget**: ~13 dev-days of work. Even with parallel execution, end-to-end completion ~3-5 wall-clock days.
- **Audit Wave findings**: if T10/T11/T12 flag critical issues → ad-hoc fixer + re-review (max 3 rounds). Could add 1-2 days if findings are substantial.

## Completion Criteria

- All 15 tasks `status: done`
- `decisions.md` has entries for each task
- T15 post-deploy QA confirms: `mcp.mnemonik.xyz/health` returns 200; live OAuth flow works end-to-end on Cursor + Claude.ai Pro; Smithery listing live; security spot-checks pass (anonymous → 401, cross-tenant isolated, rate limit 429 above threshold)
- User-spec success metrics measurable: install counter wired up

## User Approval Required

This plan involves real production deployment. Before proceeding to Phase 2 (Wave 1 execution), the user must confirm:
- (a) Approve plan as-is, OR
- (b) Modify scope (e.g., skip T14 deploy, run only Waves 1-4 as a "develop locally + manual deploy later" path)
- (c) Reject — reconsider strategy

Default proposal: option (b) — execute Waves 1-4 autonomously (code + tests + audit), then pause for user review BEFORE T13/T14/T15 (QA + deploy). This avoids autonomous production-infra changes while maximizing dev-time value.
