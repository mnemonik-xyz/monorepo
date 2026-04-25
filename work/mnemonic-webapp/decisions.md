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

## Task 2: RAG seeding -- whitepaper chunking + sign_memory + artifact generation

Created `mcp/src/seed.rs` implementing the startup seeding routine: parses `docs/WHITEPAPER.md` at `## ` headers (with h3 sub-splitting for sections exceeding ~500 tokens), calls `sign_memory()` per chunk with tags `["protocol-knowledge", "whitepaper"]`, and generates a `.zip` artifact containing `knowledge.md` (YAML frontmatter with content_hash/signer_pubkey/timestamp per chunk) and `knowledge.json` sidecar. Added `zip` crate to `mcp/Cargo.toml`. Seeder is called from `main.rs` after McpState init; skips if `store.count() > 0` (idempotent). The canonical `.zip` path is stored in a new `McpState.artifact_zip_path` field for the future `/download-knowledge` handler. Key fix during review: the h2 parser incorrectly matched `### ` lines (since they start with `## `) -- fixed with explicit exclusion and regression test. 13 unit tests cover parsing, splitting, and artifact generation.
