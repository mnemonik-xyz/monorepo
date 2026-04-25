# Decisions Log: mnemonic-webapp

## Task 1: Extend MCP config with Ollama env vars

Added `OLLAMA_URL`, `OLLAMA_MODEL`, and `RAG_CHUNK_DIR` to `config.rs` with corresponding `McpState` fields in `mcp.rs`. OLLAMA_URL is validated at startup using the `url` crate against a strict hostname whitelist (only `localhost` and `ollama` allowed, IP addresses rejected) per Decision 8 (SSRF prevention). Nine unit tests cover the validation logic including edge cases like loopback IP bypass attempts.

## Task 3: POST /chat + rate limiting + GET /download-knowledge

Created `mcp/src/chat.rs` with three capabilities:

1. **POST /chat**: Validates message (max 2000 chars via `chars().count()`, rejects empty/missing with 400 `invalid_input`). Locks store, calls `tools::recall()` for top-3 chunks, drops lock before any `.await`. Builds prompt with system instruction + context + `[USER_QUERY]...[/USER_QUERY]` delimiters (Decision 10). POSTs to Ollama `/api/generate` with `stream: false` using reqwest client with `redirect(Policy::none())` (Decision 8 SSRF prevention). Returns `{"response": "..."}` on success. Error schema per Decision 9: `{"error": "...", "code": "..."}` with codes `rate_limited`/429, `invalid_input`/400, `service_unavailable`/503, `internal_error`/500.

2. **Rate limiting**: Added `governor` crate to `Cargo.toml`. Per-IP keyed rate limiter (10 req/min) stored in `McpState.chat_limiter`, checked at the start of the `/chat` handler. Returns 429 with `{"error": "Rate limit exceeded", "code": "rate_limited"}`. Note: `tower_governor` was not used because v0.5 depends on axum 0.7 (project uses axum 0.8); instead, governor is used directly via `check_key()` in the handler.

3. **GET /download-knowledge**: Serves pre-built `.zip` from `McpState.artifact_zip_path`. Returns 404 if None or file missing. Sets `Content-Type: application/zip` and `Content-Disposition: attachment`. Changed `axum::serve` to use `into_make_service_with_connect_info::<SocketAddr>()` so `ConnectInfo` extractor works for IP-based rate limiting.

## Task 4: Initialize webapp project (React + Vite + Tailwind)

Scaffolded `webapp/` directory using Vite + React 19 + TypeScript + Tailwind CSS v4. Used the CSS-first `@theme` approach for color tokens (Tailwind v4 convention) while also providing a `tailwind.config.js` referenced via `@config` directive for backward compatibility. Theme colors match UX guidelines exactly: `#0A0F1E` background, `#00D4B4` primary accent, `#9945FF` secondary accent, `#FFFFFF`/`#8B9BC0` text, `#FF4747` error, `#00CC88` success. Playwright configured with chromium project and dev server integration. Vite proxies `/api` to `localhost:3000` for backend integration. No `npm install` was run -- consumers must run `npm install` before first use.

## Task 5: Landing page with protocol description

Created `webapp/src/components/LandingPage.tsx` with three sections: header (title + subtitle), protocol description (derived from whitepaper abstract, lines 9-18), and navigation buttons ("Start chat" and "Download protocol knowledge"). Technical terms (`recall`, `MCP`, `HTTP`, `stdio`, `memory signing`) rendered in monospace font with accent color per UX guidelines. Download button links to `/api/download-knowledge` which Vite proxies to the backend at `localhost:3000`. Modified `webapp/src/App.tsx` to use state-based view switching (`View = "landing" | "chat"`) instead of adding react-router -- no routing library exists in dependencies and only two views are needed. Chat view is a placeholder with back-navigation link. Responsive layout uses `sm:` breakpoints for button row and `max-w-2xl` content constraint. Dark theme uses existing CSS custom properties from `index.css`. Accessibility: `aria-label` on protocol description section, `type="button"` on interactive elements.

## Task 6: Chat interface with session limits and error handling

Created `webapp/src/components/ChatPage.tsx` and `webapp/src/lib/api.ts`.

**ChatPage.tsx**: Full-screen chat layout with header (back link + message counter), scrollable message area with user/bot bubbles (right-aligned user, left-aligned bot), text input with Enter-to-send, and Send button. Empty state shows "Ask a question about the Mnemonic Protocol." Loading state shows "Processing..." bubble. Client-side session counter via `useState` enforces a 50-message limit; when reached, displays "Session limit reached. Start a new session to continue." in an error-colored banner with `role="alert"` and disables input. Error messages render in a distinct red-bordered bubble. Technical content (hashes, tx IDs) auto-detected and rendered in monospace; inline backtick code rendered as `<code>` elements with accent-primary color. Dark theme using project color tokens throughout.

**api.ts**: Typed API client for `POST /chat` with `ChatRequest`/`ChatResponse`/`ChatError` interfaces matching Decision 9 schema. Auto-retry on 5xx errors (3 attempts, exponential backoff: 1s, 2s, 4s). Non-retryable errors (400, 429) throw immediately. Error code mapping: `invalid_input` -> "Invalid input.", `rate_limited` -> "Rate limit exceeded. Wait before sending another request.", `service_unavailable` -> "Service temporarily unavailable. Try again later." Network errors caught separately with "Network error. Check your connection." `ChatApiError` class carries structured `code` and `status` fields.

**Vite proxy**: Added `/chat` proxy rule alongside existing `/api` rule so dev server forwards chat requests to `http://localhost:3000`. Fetch URL uses `/chat` to match the backend route directly (no `/api` prefix).

## Task 7: Docker Compose + nginx config + Ollama model

Created three-service Docker Compose setup for single-server deployment:

1. **docker-compose.yml**: Three services (nginx, mcp, ollama) on a shared `mnemonic` bridge network. nginx serves static React build from `webapp/dist/` and reverse-proxies to MCP. MCP service uses the existing root `Dockerfile` with env vars for Ollama URL (`http://ollama:11434`), model, and RAG config. Ollama service uses a custom `ollama/Dockerfile`. Keypair mounted as read-only volume at `/run/secrets/keypair` (host directory `./keypair` must have 400 permissions). Service dependency chain: ollama (healthy) -> mcp (healthy) -> nginx. MCP healthcheck uses bash `/dev/tcp` probe (no curl in slim Debian image). Removed deprecated `version` field per Compose V2 spec.

2. **nginx.conf**: Proxies only `/mcp`, `/chat`, `/download-knowledge`, `/health` to MCP backend (Decision 13: `/admin/stats` and all `/admin` paths return 403). Security hardening: `server_tokens off`, `X-Frame-Options SAMEORIGIN`, `X-Content-Type-Options nosniff`, `client_max_body_size 64k`. Chat endpoint gets 60s proxy_read_timeout (LLM inference can take 5-15s). SPA fallback via `try_files $uri $uri/ /index.html`. ACME challenge location for certbot (Decision 12). HTTPS server block is commented out as a template for post-certbot activation.

3. **ollama/Dockerfile**: Multi-stage build -- first stage starts ollama server and pulls `qwen2.5:3b` at build time (Decision 11), second stage copies model data and installs curl for healthchecks. Custom `entrypoint.sh` starts ollama, waits for readiness, issues a warm-up inference (`num_predict:1`) to load the model into memory, then `wait`s on the server PID.

Rate limiting is not duplicated at the nginx level -- the MCP server's governor-based per-IP limiter (10 req/min on `/chat`) is the single source of truth for rate limiting (per Task 3 decisions).

## Task 8: Playwright E2E tests

Created `webapp/e2e/chat.spec.ts` with 6 E2E tests covering all critical user flows, fully mocked via `page.route()` (no live backend dependency):

1. **Golden path**: Landing page visible, download link verified (href, download attribute, Playwright download event with suggestedFilename), navigate to chat, send question, user message and bot answer appear in `role="log"` region, counter shows 1/50.

2. **Out-of-scope rejection**: Send off-topic question, verify rejection message appears in chat.

3. **Session limit**: Send 49 mocked messages (instant fulfillment via route mock) to reach counter=49, then send 50th. Verify `role="alert"` banner with "Session limit reached" text, input textarea disabled, Send button disabled.

4. **Error state (retryable 503)**: Mock `/chat` to return 503 on all attempts. Uses `page.clock.install()` + `fastForward()` to skip retry backoff delays. Verifies error message "Service temporarily unavailable. Try again later." and `callCount === 3`.

5. **Error state (non-retryable 429)**: Mock `/chat` to return 429. Verifies immediate error message and `callCount === 1` (no retries).

6. **Back navigation**: Click back button from chat, verify landing page heading reappears.

All tests use semantic selectors (`getByRole`, `getByLabel`, `getByText`). Test file placed in `webapp/e2e/` to match existing `playwright.config.ts` testDir. No production source code modified.

## Task 2: RAG seeding -- whitepaper chunking + sign_memory + artifact generation

Created `mcp/src/seed.rs` implementing the startup seeding routine: parses `docs/WHITEPAPER.md` at `## ` headers (with h3 sub-splitting for sections exceeding ~500 tokens), calls `sign_memory()` per chunk with tags `["protocol-knowledge", "whitepaper"]`, and generates a `.zip` artifact containing `knowledge.md` (YAML frontmatter with content_hash/signer_pubkey/timestamp per chunk) and `knowledge.json` sidecar. Added `zip` crate to `mcp/Cargo.toml`. Seeder is called from `main.rs` after McpState init; skips if `store.count() > 0` (idempotent). The canonical `.zip` path is stored in a new `McpState.artifact_zip_path` field for the future `/download-knowledge` handler. Key fix during review: the h2 parser incorrectly matched `### ` lines (since they start with `## `) -- fixed with explicit exclusion and regression test. 13 unit tests cover parsing, splitting, and artifact generation.
