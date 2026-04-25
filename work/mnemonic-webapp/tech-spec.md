---
created: 2026-04-25
status: draft
branch: dev
size: L
---

# Tech Spec: mnemonic-webapp (MVP Protocol Chatbot)

## Solution

Build a React webapp backed by the existing MCP HTTP server. Add two new endpoints to the MCP server: `POST /chat` (RAG chatbot) and `GET /download-knowledge` (pre-built artifact). Add a startup seeding routine that splits the whitepaper into sections and stores them as attested memory items via `sign_memory`. Deploy everything on a single VPS via Docker Compose (nginx + MCP + Ollama).

Key technical moves:
1. Add `governor` + `tower_governor` to `mcp/Cargo.toml` for rate limiting on `/chat`.
2. Add `zip` crate for artifact generation at seed time.
3. Extend `config.rs` with Ollama env vars (`OLLAMA_URL`, `OLLAMA_MODEL`).
4. Add `McpState` fields for Ollama config.
5. Implement RAG seeding in `main.rs` startup: parse `WHITEPAPER.md` by `##` headers, call `sign_memory` per section, generate `.zip` artifact to disk.
6. Implement `/chat` handler: embed query via `recall()`, build prompt with system instruction + context, POST to Ollama `/api/generate`, return response.
7. Implement `/download-knowledge` handler: serve pre-built `.zip` file.
8. Create `webapp/` React app: landing page + chat UI (dark theme per UX guidelines).
9. Create `docker-compose.yml` with nginx, MCP server, and Ollama services.

## Architecture

### What we're building/modifying

- **`webapp/` (new)** -- React + Vite + Tailwind CSS app. Landing page with protocol description, chat interface with message history, download button for knowledge artifact. Dark theme (#0A0F1E background, #00D4B4 accent).
- **`mcp/src/main.rs` (modified)** -- Add `/chat` and `/download-knowledge` routes to axum Router. Add RAG seeding startup logic.
- **`mcp/src/chat.rs` (new)** -- Chat handler: recall + prompt building + Ollama HTTP call.
- **`mcp/src/seed.rs` (new)** -- Whitepaper chunking, `sign_memory` calls, artifact `.zip` generation.
- **`mcp/src/config.rs` (modified)** -- Add Ollama env vars.
- **`mcp/src/mcp.rs` (modified)** -- Add Ollama config fields to `McpState`.
- **`docker-compose.yml` (new)** -- Three services: nginx, mcp, ollama.
- **`nginx.conf` (new)** -- Serve static React build, reverse proxy `/chat`, `/mcp`, `/download-knowledge` to MCP.

### How it works

```
Browser (React webapp)
  │
  ├── GET /                     → nginx → static React build (landing + chat UI)
  ├── POST /chat                → nginx → MCP server → recall() + Ollama → response
  ├── GET /download-knowledge   → nginx → MCP server → pre-built .zip file
  └── POST /mcp                 → nginx → MCP server → existing JSON-RPC tools
```

**Chat flow:**
1. User sends message → `POST /chat {"message": "...", "session_id": "..."}`
2. Handler validates message length (max 2000 chars)
3. `recall()` embeds query, finds top-3 similar knowledge chunks from SQLite
4. Build prompt: system instruction + recalled context + user message
5. POST to Ollama `http://ollama:11434/api/generate` with model `qwen2.5:3b`
6. Return `{"response": "..."}`

**Seeding flow (startup):**
1. Check `store.count(&pubkey)` -- if > 0, skip
2. Parse `docs/WHITEPAPER.md` by `##` section headers
3. For each section: call `sign_memory()` with tags `["protocol-knowledge", "whitepaper"]`
4. Generate `.md` artifact (YAML frontmatter + all section content) and `.json` sidecar
5. Bundle into `.zip`, save to disk at configured path

### Shared resources

| Resource | Owner | Consumers | Instance count |
|----------|-------|-----------|----------------|
| `SqliteStore` (Mutex-wrapped) | `McpState` in main.rs | `/chat` (recall), `/mcp` (all tools), seed.rs | 1 |
| `Box<dyn Embedder>` | `McpState` | `/chat` (recall), seed.rs (sign_memory) | 1 |
| Ollama HTTP client | `reqwest::Client` in chat.rs | `/chat` handler | 1 (reused) |

## Decisions

### Decision 1: RAG via existing recall() -- no new storage API
**Decision:** Use the existing `tools::recall()` function for the chat RAG pipeline. Pre-build the downloadable artifact at seed time and save to disk -- serve as static file from `/download-knowledge`. No `find_by_tag()` method needed.
**Rationale:** `recall()` already embeds queries and does cosine search over attested content. Pre-building the artifact avoids adding new storage trait methods to core/. Supports US-2 (download artifact) and US-1 (chatbot answers based on protocol knowledge).
**Alternatives considered:** Add `find_by_tag()` to `AttestationStore` -- rejected as unnecessary for MVP (artifact is static, generated once at seed time).

### Decision 2: Rate limiting with governor crate
**Decision:** Add `governor` + `tower_governor` to `mcp/Cargo.toml`. Apply rate-limit middleware only to `/chat` route (10 req/min per IP). The existing `/mcp` endpoint is not rate-limited (has its own payment gating).
**Rationale:** `governor` is the idiomatic Rust rate-limiting solution for tower/axum. Per-route application avoids interfering with existing MCP tool access. Supports US-4 (rate limit: 11th request returns 429).
**Alternatives considered:** Manual DashMap-based counter -- rejected as reinventing the wheel. Global rate limit on all routes -- rejected because `/mcp` has payment gating.

### Decision 3: Session limit client-side only
**Decision:** Track message count in React state (useState counter). No server-side session tracking. Show "Session limit reached" at 50 messages. Page refresh resets.
**Rationale:** Server-side session state adds complexity (HashMap or SQLite table) with no security benefit for an open-access MVP. Client-side is simpler and matches the "no persist between visits" constraint. Supports US-4 (session limit). `[TECHNICAL]`
**Alternatives considered:** Server-side session tracking with in-memory HashMap -- rejected as unnecessary for MVP with open access.

### Decision 4: Ollama non-streaming response
**Decision:** Use Ollama `/api/generate` with `stream: false`. Return full response in a single JSON body from `/chat`.
**Rationale:** Streaming (SSE) improves perceived performance but adds complexity to both backend (axum::response::Sse) and frontend (EventSource parsing). For a 3B model with 5-15s response time, non-streaming is acceptable for MVP. Supports US-1 (15-second response time). `[TECHNICAL]`
**Alternatives considered:** SSE streaming -- noted as post-MVP improvement for better UX.

### Decision 5: Whitepaper chunking by ## headers
**Decision:** Split `docs/WHITEPAPER.md` at `## ` (h2) headers. Each section becomes one attested memory item. Sections exceeding ~500 tokens are further split at `### ` (h3) level. Target: max ~400 tokens per chunk.
**Rationale:** The whitepaper has 17 sections, most 10-30 lines. Recall returns top-3 chunks; with 3 chunks at ~400 tokens each = ~1200 tokens of context, leaving ~2800 tokens for system prompt + user message + response within Qwen2.5-3B's ~4K context window. Supports US risk 3 mitigation. `[TECHNICAL]`
**Alternatives considered:** Fixed token-size chunking -- rejected because section boundaries are semantically meaningful.

### Decision 6: Docker Compose single-server deploy
**Decision:** Docker Compose with three services: nginx (static files + reverse proxy), mcp (Rust binary), ollama (Qwen2.5-3B). Single VPS on justhost.asia (6+ vCPU, 12GB+ RAM). Keypair mounted as read-only volume.
**Rationale:** Single server is simplest for MVP. All services communicate over Docker network (localhost). Ollama cold start mitigated by health check + warm-up request in Compose startup. Supports US deploy requirement (docker compose up).
**Alternatives considered:** Separate static hosting (Cloudflare Pages) + VPS for backend -- adds complexity for no MVP benefit.

### Decision 7: Pre-built artifact served as static file
**Decision:** Generate the `.zip` artifact (Markdown + JSON sidecar) during the seeding step. Save to disk. `GET /download-knowledge` serves the file directly. No runtime SQLite queries.
**Rationale:** The artifact content is static (whitepaper doesn't change at runtime). Serving a pre-built file is simpler and faster than dynamically assembling from database. Supports US-2 (download artifact).
**Alternatives considered:** Dynamic assembly from SQLite -- rejected because it requires a `find_by_tag()` method not in current storage trait, and content is static anyway.

## Testing Strategy

**Size L -- full test pyramid.**

**Unit tests (Rust):**
- `/chat` handler with mocked Ollama + mocked recall (reqwest mock + injected recall results)
- Seed module: whitepaper parsing, chunk splitting, artifact generation (verify .md/.json structure)
- Rate limiter: injectable counter, verify 429 after threshold
- Input validation: message > 2000 chars returns 400

**Unit tests (React):**
- Chat component: message send/receive, session counter, limit enforcement
- Landing page: renders protocol description, "Start chat" button
- Download button: triggers correct URL

**Integration tests (Rust):**
- Full RAG pipeline: seed whitepaper → recall query → verify relevant content returned
- Rate limit: 11 curl requests → 429 on 11th

**E2E tests (Playwright):**
- Golden path: open site → click "Start chat" → send question → receive answer → download artifact
- Out-of-scope question: ask irrelevant question → get rejection
- Session limit: send 50 messages → see limit notification
- Error state: stop Ollama → send request → see error message

## Agent Verification Plan

| Step | Tool | Expected Result |
|------|------|-----------------|
| 1. Docker Compose up | `bash: docker compose up -d && docker compose ps` | 3 services running |
| 2. Health check | `bash: curl http://localhost:3000/health` | `{"status":"ok"}` |
| 3. Ollama warm-up | `bash: curl http://localhost:11434/api/tags` | Model `qwen2.5:3b` listed |
| 4. Knowledge seeded | `bash: curl -s -X POST http://localhost:3000/mcp -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemonic_whoami","arguments":{}}}'` | `attestation_count > 0` |
| 5. Chat works | `bash: curl -s -X POST http://localhost:3000/chat -H 'Content-Type: application/json' -d '{"message":"What is Mnemonic Protocol?","session_id":"test"}'` | JSON with `response` field containing protocol info |
| 6. Artifact download | `bash: curl -s -o knowledge.zip http://localhost:3000/download-knowledge && unzip -l knowledge.zip` | Contains `.md` and `.json` files |
| 7. Rate limit | `bash: for i in $(seq 12); do curl -s -o /dev/null -w "%{http_code}\n" -X POST http://localhost:3000/chat -H 'Content-Type: application/json' -d '{"message":"test","session_id":"rl"}'; done` | 11th request returns 429 |
| 8. Input validation | `bash: curl -s -w "%{http_code}" -X POST http://localhost:3000/chat -H 'Content-Type: application/json' -d "{\"message\":\"$(python3 -c 'print("x"*2001)')\",\"session_id\":\"test\"}"` | HTTP 400 |
| 9. Playwright E2E | `bash: cd webapp && npx playwright test` | All tests pass |

## Implementation Tasks

### Wave 1: Backend -- Config & Seeding

**Task 1: Extend MCP config with Ollama env vars**
Add `OLLAMA_URL`, `OLLAMA_MODEL`, `RAG_CHUNK_DIR` (artifact output path) to `config.rs`. Add corresponding fields to `McpState` in `mcp.rs`. No behavioral changes.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Files to modify: `mcp/src/config.rs`, `mcp/src/mcp.rs`, `.env.example`
- Files to read: `mcp/src/main.rs`

**Task 2: RAG seeding -- whitepaper chunking + sign_memory + artifact generation**
Implement `mcp/src/seed.rs`: parse whitepaper by `##` headers, call `sign_memory()` per chunk with `protocol-knowledge` tag, generate `.md` (YAML frontmatter) + `.json` sidecar, bundle into `.zip`. Add `zip` crate to `mcp/Cargo.toml`. Call seeder from `main.rs` startup after McpState is initialized. Skip if `count() > 0`.

- Skill: `code-writing`
- Reviewers: `code-reviewer`, `test-reviewer`
- Verify-smoke: `cargo run -p mnemonic-mcp` starts, seeds whitepaper, creates `.zip` artifact on disk
- Files to modify: `mcp/src/main.rs`, `mcp/Cargo.toml`
- Files to create: `mcp/src/seed.rs`
- Files to read: `mcp/src/tools.rs` (sign_memory), `docs/WHITEPAPER.md`

### Wave 2: Backend -- Chat & Download Endpoints

**Task 3: POST /chat endpoint with RAG + Ollama**
Implement `mcp/src/chat.rs`: validate message length (max 2000 → 400), recall top-3 chunks, build prompt (system instruction + context + user message), POST to Ollama `/api/generate` with `stream: false`, return `{"response": "..."}`. Add route to axum Router in `main.rs`.

- Skill: `code-writing`
- Reviewers: `code-reviewer`, `security-auditor`
- Verify-smoke: `curl -X POST http://localhost:3000/chat -d '{"message":"What is Mnemonic?","session_id":"t"}'` returns relevant answer
- Files to modify: `mcp/src/main.rs`
- Files to create: `mcp/src/chat.rs`
- Files to read: `mcp/src/tools.rs` (recall), `mcp/src/mcp.rs` (McpState)

**Task 4: Rate limiting on /chat**
Add `governor` + `tower_governor` to `mcp/Cargo.toml`. Apply rate-limit middleware (10 req/min per IP) to `/chat` route only. Return HTTP 429 with JSON body when exceeded.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Verify-smoke: 11 rapid curl requests to `/chat` → last returns 429
- Files to modify: `mcp/src/main.rs`, `mcp/Cargo.toml`

**Task 5: GET /download-knowledge endpoint**
Serve the pre-built `.zip` artifact from disk. Return 404 if artifact not yet generated (seeding hasn't run). Set `Content-Type: application/zip` and `Content-Disposition: attachment`.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Verify-smoke: `curl -o knowledge.zip http://localhost:3000/download-knowledge && unzip -l knowledge.zip`
- Files to modify: `mcp/src/main.rs`

### Wave 3: Frontend -- React Webapp

**Task 6: Initialize webapp project (React + Vite + Tailwind)**
Create `webapp/` directory with Vite + React + TypeScript + Tailwind CSS. Configure dark theme colors from UX guidelines (#0A0F1E, #00D4B4, #9945FF). Add Playwright as dev dependency.

- Skill: `infrastructure-setup`
- Reviewers: `code-reviewer`
- Files to create: `webapp/package.json`, `webapp/vite.config.ts`, `webapp/tailwind.config.js`, `webapp/src/`, `webapp/playwright.config.ts`

**Task 7: Landing page**
React landing page component with protocol description (static copy based on whitepaper abstract), "Start chat" button, and "Download protocol knowledge" button (always visible). Dark theme, responsive layout.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Verify-user: Open localhost:5173, verify landing page renders with protocol description and both buttons
- Files to modify: `webapp/src/App.tsx`
- Files to create: `webapp/src/components/LandingPage.tsx`

**Task 8: Chat interface**
Chat component with message list, input field, send button. Session counter (useState, max 50). Auto-retry on error (2-3 attempts). Error/limit messages per UX guidelines. Calls `POST /chat` on backend.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Verify-user: Open chat, send a question about the protocol, verify answer appears. Send 50 messages, verify limit shown.
- Files to create: `webapp/src/components/ChatPage.tsx`, `webapp/src/lib/api.ts`

### Wave 4: Infrastructure -- Docker Compose & Deploy

**Task 9: Docker Compose + nginx config**
Create `docker-compose.yml` with three services: nginx (serves static React build + reverse proxy to MCP), mcp (existing Dockerfile), ollama (official image with Qwen2.5-3B). Create `nginx.conf` for static serving + proxy. Add Ollama warm-up health check. Mount keypair as read-only volume.

- Skill: `deploy-pipeline`
- Reviewers: `code-reviewer`, `security-auditor`
- Verify-smoke: `docker compose up -d && docker compose ps` shows 3 services healthy
- Files to create: `docker-compose.yml`, `nginx.conf`
- Files to read: `Dockerfile`

**Task 10: Playwright E2E tests**
E2E tests: golden path (landing → chat → answer → download), out-of-scope question rejection, session limit (50 messages), error state (Ollama down).

- Skill: `code-writing`
- Reviewers: `test-reviewer`
- Verify-smoke: `cd webapp && npx playwright test` passes
- Files to create: `webapp/tests/e2e/chat.spec.ts`

### Wave 5: Audit

**Task 11: Code Audit**
Holistic code quality review of all new code (chat.rs, seed.rs, chat handler, React components, Docker config).

- Skill: `code-reviewing`
- Reviewers: none

**Task 12: Security Audit**
OWASP Top 10 review: input validation, rate limiting, CORS, keypair handling, Ollama proxy, Docker security.

- Skill: `security-auditor`
- Reviewers: none

**Task 13: Test Audit**
Test quality and coverage: unit test adequacy, integration test coverage, E2E scenario completeness.

- Skill: `test-master`
- Reviewers: none

### Wave 6: Final

**Task 14: Pre-deploy QA**
Run full test suite, verify all acceptance criteria from user-spec. Produce structured QA report.

- Skill: `pre-deploy-qa`
- Reviewers: none

**Task 15: Deploy to justhost.asia**
Deploy Docker Compose to VPS. Configure domain/SSL. Verify all endpoints work in production.

- Skill: `deploy-pipeline`
- Reviewers: none
- Verify-smoke: `curl https://<domain>/health` returns ok; `curl https://<domain>/chat` works
- Verify-user: Open site in browser, test chat, download artifact

## User-Spec Deviations

None. All decisions align with user-spec requirements.
