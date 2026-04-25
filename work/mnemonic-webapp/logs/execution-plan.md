# Execution Plan: mnemonic-webapp

## Waves

### Wave 1: Backend Config + Seeding (sequential)
| Task | Name | Skill | Reviewers | Verify |
|------|------|-------|-----------|--------|
| 1 | Extend MCP config with Ollama env vars | code-writing | code-reviewer | - |
| 2 | RAG seeding (whitepaper chunking + artifact) | code-writing | code-reviewer, test-reviewer | smoke |

**Note:** Task 2 depends on Task 1. Execute sequentially: 1 first, then 2.

### Wave 2: Backend Chat Endpoint
| Task | Name | Skill | Reviewers | Verify |
|------|------|-------|-----------|--------|
| 3 | POST /chat + rate limiting + download endpoint | code-writing | code-reviewer, security-auditor | smoke |

### Wave 3: Frontend (Task 4 first, then 5+6 parallel)
| Task | Name | Skill | Reviewers | Verify |
|------|------|-------|-----------|--------|
| 4 | Initialize webapp (React + Vite + Tailwind) | infrastructure-setup | code-reviewer | - |
| 5 | Landing page | code-writing | code-reviewer | user |
| 6 | Chat interface | code-writing | code-reviewer | user |

**Note:** Tasks 5 and 6 depend on Task 4. Execute 4 first, then 5+6 in parallel.

### Wave 4: Infrastructure + E2E (parallel)
| Task | Name | Skill | Reviewers | Verify |
|------|------|-------|-----------|--------|
| 7 | Docker Compose + nginx + Ollama | deploy-pipeline | code-reviewer, security-auditor | smoke |
| 8 | Playwright E2E tests | code-writing | test-reviewer | smoke |

### Wave 5: Audit (parallel)
| Task | Name | Skill | Reviewers | Verify |
|------|------|-------|-----------|--------|
| 9 | Code Audit | code-reviewing | none | - |
| 10 | Security Audit | security-auditor | none | - |
| 11 | Test Audit | test-master | none | - |

### Wave 6: Final (sequential)
| Task | Name | Skill | Reviewers | Verify |
|------|------|-------|-----------|--------|
| 12 | Pre-deploy QA | pre-deploy-qa | none | - |
| 13 | Deploy to justhost.asia | deploy-pipeline | none | smoke, user |

## User Checks
- After Wave 3: verify landing page and chat UI on localhost
- After Wave 6 (Task 13): verify production site, test chat, download artifact, load .md in ChatGPT
