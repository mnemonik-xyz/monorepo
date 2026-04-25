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
**Decision:** Generate the `.zip` artifact (Markdown + JSON sidecar) during the seeding step. Save to disk. `GET /download-knowledge` serves the file directly. No runtime SQLite queries. Artifact `.md` YAML frontmatter must include fields: `content_hash`, `signer_pubkey`, `timestamp`. Artifact `.json` sidecar includes same fields plus `arweave_tx` (or `local:*` in local mode). Artifact path is resolved to an absolute canonical path at startup and stored in `McpState` -- handler serves only that file, never a user-supplied filename.
**Rationale:** The artifact content is static (whitepaper doesn't change at runtime). Serving a pre-built file is simpler and faster than dynamically assembling from database. Supports US-2 (download artifact).
**Alternatives considered:** Dynamic assembly from SQLite -- rejected because it requires a `find_by_tag()` method not in current storage trait, and content is static anyway.

### Decision 8: OLLAMA_URL whitelist validation + no-redirect HTTP client
**Decision:** Validate `OLLAMA_URL` at startup -- must match `http://localhost:*` or `http://ollama:*`. Reject any other URL with a fatal error. The `reqwest::Client` used for Ollama calls must be configured with `redirect(Policy::none())` to prevent SSRF via redirects. `[TECHNICAL]`
**Rationale:** OLLAMA_URL is read from env var. If misconfigured or tampered with, the MCP server becomes an SSRF vector against the Docker-internal network. Whitelist + no-redirect eliminates this risk.
**Alternatives considered:** No validation -- rejected due to SSRF risk.

### Decision 9: Chat API error response schema
**Decision:** All `/chat` error responses follow the format `{"error": "<message>", "code": "<error_code>"}`. Codes: `rate_limited` (429), `invalid_input` (400), `service_unavailable` (503), `internal_error` (500). Supports US-4 (rate limit/error messages). `[TECHNICAL]`
**Rationale:** Structured error responses enable the React frontend to display appropriate user-facing messages per UX guidelines.
**Alternatives considered:** Plain text errors -- rejected because frontend needs to parse error types for different UI states.

### Decision 10: Prompt injection mitigation
**Decision:** The system prompt explicitly instructs the model to ignore any instructions embedded in user messages. User message is placed in a clearly delimited section (`[USER_QUERY]...[/USER_QUERY]`). No HTML/JS content from user reaches the DOM without sanitization. `[TECHNICAL]`
**Rationale:** Without prompt-delimiter escaping, a crafted message could hijack the system prompt and bypass topic restrictions.
**Alternatives considered:** Full input sanitization/stripping -- rejected as too aggressive for legitimate technical questions about the protocol.

### Decision 11: Ollama model pull strategy
**Decision:** Use a custom Ollama Dockerfile that pulls `qwen2.5:3b` at build time (`ollama pull qwen2.5:3b` in a build script). The compose entrypoint issues a warm-up inference request before accepting traffic. `[TECHNICAL]`
**Rationale:** The official Ollama Docker image ships without any model. Without pre-pulling, first `docker compose up` fails or has a multi-minute delay downloading 2GB+ model.
**Alternatives considered:** Pull at runtime in entrypoint -- slower first start but simpler. Noted as acceptable fallback.

### Decision 12: SSL/TLS via Let's Encrypt
**Decision:** Use certbot with nginx plugin for SSL certificate provisioning. The nginx service handles TLS termination. Certificate renewal runs as a cron job inside the nginx container (or a sidecar certbot container). `[TECHNICAL]`
**Rationale:** HTTPS is required for production. Let's Encrypt is free and automated.
**Alternatives considered:** Cloudflare proxy for SSL -- adds a dependency; manual certs -- doesn't auto-renew.

### Decision 13: Block /admin/stats at nginx level
**Decision:** nginx config does NOT proxy `/admin/stats` to the MCP server. Only `/mcp`, `/chat`, `/download-knowledge`, and `/health` are proxied. `[TECHNICAL]`
**Rationale:** `/admin/stats` exposes pricing/revenue data. With CORS wildcard on the MCP server, it would be browser-accessible from any origin. Blocking at nginx is the simplest fix.
**Alternatives considered:** Add auth header check on `/admin/stats` -- more complex, unnecessary for MVP.

## Testing Strategy

**Size L -- full test pyramid.**

**Unit tests (Rust):**
- `/chat` handler with mocked Ollama (httpmock) + mocked recall: success path, Ollama timeout/error → 503, empty recall results → graceful degradation
- Seed module: whitepaper parsing, chunk splitting (including h3 sub-split for large sections, empty section edge case), artifact generation (verify .md YAML frontmatter contains content_hash/signer_pubkey/timestamp, .json structure valid)
- Seed idempotency: second call with count() > 0 skips without error
- Rate limiter: injectable clock/counter (not timing-dependent), verify 429 after threshold
- Input validation: message > 2000 chars → 400, exactly 2000 chars → 200, empty message → 400, missing message field → 400
- Download handler: artifact exists → 200 with zip, artifact missing → 404

**Unit tests (React):**
- Chat component: message send/receive, session counter, limit enforcement at 50, retry on error (2-3 attempts with backoff)
- Landing page: renders protocol description, "Start chat" button
- Download button: triggers correct URL

**Integration tests (Rust):**
- Full RAG pipeline: seed whitepaper → recall query "What are the 5 MCP tools?" → verify response contains all five tool names
- Rate limit: pre-saturated governor bucket, next request returns 429 (deterministic, no timing dependency)
- Ollama error propagation: mock Ollama returning 500 → chat handler returns 503 with structured error JSON

**E2E tests (Playwright):**
- Golden path: open site → click "Start chat" → send question → receive answer → download artifact
- Out-of-scope question: ask irrelevant question → get rejection message
- Session limit: inject initial counter state at 49, send 1 message → see limit notification (avoids 50 real round-trips)
- Error state: stop Ollama → send request → see error message after retry

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

### Wave 2: Backend -- Chat Endpoint

**Task 3: POST /chat endpoint with RAG + Ollama + rate limiting + download endpoint**
Implement chat handler, rate limiting, and download endpoint. Chat handler: validate input, recall top-3 chunks, build prompt with system instruction and delimited user query, call Ollama, return structured response/error JSON. Rate limit via governor (10 req/min per IP on /chat only). Download handler serves pre-built .zip artifact. Add all three routes to axum Router.

- Skill: `code-writing`
- Reviewers: `code-reviewer`, `security-auditor`
- Verify-smoke: `curl -X POST http://localhost:3000/chat -d '{"message":"What is Mnemonic?","session_id":"t"}'` returns relevant answer
- Files to modify: `mcp/src/main.rs`, `mcp/Cargo.toml`
- Files to create: `mcp/src/chat.rs`
- Files to read: `mcp/src/tools.rs` (recall), `mcp/src/mcp.rs` (McpState)

### Wave 3: Frontend -- React Webapp (sequential: Task 4 first, then 5+6 parallel)

**Task 4: Initialize webapp project (React + Vite + Tailwind)**
Scaffold webapp/ with Vite + React + TypeScript + Tailwind CSS. Configure dark theme colors from UX guidelines. Add Playwright as dev dependency.

- Skill: `infrastructure-setup`
- Reviewers: `code-reviewer`
- Files to create: `webapp/package.json`, `webapp/vite.config.ts`, `webapp/tailwind.config.js`, `webapp/src/`, `webapp/playwright.config.ts`

**Task 5: Landing page**
Landing page component with protocol description, "Start chat" button, and "Download protocol knowledge" button (always visible). Dark theme, responsive layout.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Verify-user: Open localhost:5173, verify landing page renders with protocol description and both buttons
- Files to modify: `webapp/src/App.tsx`
- Files to create: `webapp/src/components/LandingPage.tsx`

**Task 6: Chat interface**
Chat component with message list, input field, send button. Client-side session counter (max 50) with auto-retry on error (2-3 attempts, exponential backoff). Structured error display per UX guidelines. Calls `POST /chat` on backend.

- Skill: `code-writing`
- Reviewers: `code-reviewer`
- Verify-user: Open chat, send a question about the protocol, verify answer appears.
- Files to create: `webapp/src/components/ChatPage.tsx`, `webapp/src/lib/api.ts`

### Wave 4: Infrastructure -- Docker Compose & E2E

**Task 7: Docker Compose + nginx config + Ollama model**
Create `docker-compose.yml` with three services: nginx (static React build + reverse proxy, blocks /admin/stats), mcp (existing Dockerfile), ollama (custom Dockerfile that pre-pulls qwen2.5:3b). Create `nginx.conf` proxying only /mcp, /chat, /download-knowledge, /health. Warm-up health check on Ollama. Mount keypair as read-only volume (chmod 400).

- Skill: `deploy-pipeline`
- Reviewers: `code-reviewer`, `security-auditor`
- Verify-smoke: `docker compose up -d && docker compose ps` shows 3 services healthy
- Files to create: `docker-compose.yml`, `nginx.conf`, `ollama/Dockerfile`
- Files to read: `Dockerfile`

**Task 8: Playwright E2E tests**
E2E tests covering golden path, out-of-scope question rejection, session limit (inject counter at 49 to avoid 50 round-trips), and error state (Ollama down).

- Skill: `code-writing`
- Reviewers: `test-reviewer`
- Verify-smoke: `cd webapp && npx playwright test` passes
- Files to create: `webapp/tests/e2e/chat.spec.ts`

### Wave 5: Audit

**Task 9: Code Audit**
Holistic code quality review of all new code (chat.rs, seed.rs, React components, Docker config).

- Skill: `code-reviewing`
- Reviewers: none

**Task 10: Security Audit**
OWASP Top 10 review: input validation, prompt injection mitigation, rate limiting, CORS, OLLAMA_URL whitelist, keypair handling, nginx proxy restrictions.

- Skill: `security-auditor`
- Reviewers: none

**Task 11: Test Audit**
Test quality and coverage: unit test adequacy (including edge cases), integration test determinism, E2E scenario completeness.

- Skill: `test-master`
- Reviewers: none

### Wave 6: Final

**Task 12: Pre-deploy QA**
Run full test suite, verify all acceptance criteria from user-spec including benchmark question ("What are the 5 MCP tools?" must return all five). Produce structured QA report.

- Skill: `pre-deploy-qa`
- Reviewers: none

**Task 13: Deploy to justhost.asia**
Deploy Docker Compose to VPS. Configure domain + SSL via Let's Encrypt (certbot + nginx). Verify all endpoints work in production.

- Skill: `deploy-pipeline`
- Reviewers: none
- Verify-smoke: `curl https://<domain>/health` returns ok; `curl https://<domain>/chat` works
- Verify-user: Open site in browser, test chat, download artifact

## User-Spec Deviations

**session_id field in /chat API:** User-spec specifies `POST /chat` accepts `session_id`. Tech-spec Decision 3 makes session tracking client-side only, so `session_id` is accepted but unused server-side. Kept for API forward-compatibility when server-side sessions are added later. `[ACCEPTED]`
