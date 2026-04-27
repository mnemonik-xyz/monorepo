---
created: 2026-04-26
status: backlog
---

# mnemonic-integrations — Backlog (Post-Hackathon)

Всё, что вытолкнуто из Phase 1 хакатон-MVP. Последовательность не зафиксирована — приоритизация после демо на основе обратной связи.

## P1.5 — Stabilization (1-2 weeks after hackathon)

### Turnkey MPC integration
**Что:** Заменить in-browser localStorage Ed25519 keypair на email-onboarded MPC wallet через Turnkey API.
**Зачем:** Email recovery + cross-device sync + non-custodial security. Localstorage keypair — серьёзный fail mode (юзер чистит браузер = identity потеряна).
**Migration:** существующий pubkey мигрирует в Turnkey custody без смены user identity. Webapp флоу: "Migrate to recoverable identity" → email signup → Turnkey импортирует existing keypair → localStorage очищается.
**Эффорт:** ~5 dev-days.

### Docker image GHCR publish
**Что:** Расширить `.github/workflows/release.yml` — на git tag собирать и пушить `ghcr.io/mnemonik-xyz/mnemonic-mcp:latest` (и semver-теги).
**Зачем:** Self-host пользователи и агенты могут `docker pull` и запустить MCP локально без сборки из source.
**Текущее состояние:** Dockerfile есть, docker-compose есть; missing — GHCR push step в CI.
**Эффорт:** ~0.5 dev-day.

### `STORAGE_MODE=full` на хостинге
**Что:** Включить Arweave + Solana запись на `mcp.mnemonik.xyz`.
**Зачем:** Полная on-chain атестация — это differentiator протокола.
**Зависит от:** funded Solana keypair, мониторинг Irys/Arweave costs, payment-flow (см. ниже).
**Эффорт:** ~1 dev-day setup + ongoing operational cost.

### `PAYMENT_MODE=balance` activation
**Что:** Включить billing per `sign_memory` call. Bearer token / API key system уже есть в `mcp/src/payment.rs` — нужно только активировать через ENV + добавить top-up UI в webapp.
**Зачем:** Sustainability — `sign_memory` тратит Arweave + Solana fees, нужна окупаемость.
**Эффорт:** ~2 dev-days (UI + Stripe/Solana payment flow).

## P2 — Reach Expansion (2-4 weeks)

### npm publish `@mnemonic/core`
**Что:** Опубликовать WASM-сборку core/ как npm-пакет с TypeScript типами.
**Зачем:** Reusability — любой 3rd-party developer может встроить attestation logic в свой webapp/extension. Это "WASM channel" из 3-канальной модели.
**Эффорт:** ~1 dev-day (npm publish CI шаг + API documentation).

### Webapp дополнительные страницы
- **(3) Bundles browser** — список user attestations с поиском, Arweave/Solana ссылками, content preview
- **(4) Top-up balance UI** — пополнение баланса через Solana/USDC (Stripe — позже)
- **(5) Privacy/delete** — отозвать bundle (Arweave immutable, но мы можем deactivate в SQLite + UI hint)
- **(6) Stats dashboard** — твоя активность: signed/recalled count, размер атестованной памяти, share

**Эффорт:** ~3-4 dev-days.

### Additional MCP registries
- **Anthropic Connectors Directory** — partner outreach, без public submission portal. Пишем DevRel.
- **mcp.directory** — community registry, простая submission.
- **Glama** — community.
- **MCP Hub (TrueFoundry list)** — submission через GitHub PR.

**Эффорт:** ~1 dev-day total (paperwork-heavy).

### Headless Claude Code в CI
**Что:** Запустить `claude code` headless с Mnemonic MCP installed, прогнать `whoami → sign_memory → recall` сценарий, assert на output. Nightly + pre-release smoke.
**Зачем:** End-to-end доказательство что реальный модель правильно interpret-ит наши tool descriptions (MCP Inspector валидирует только schema, не behavior).
**Эффорт:** ~1 dev-day.

## P3 — Opportunistic / Risk-Heavy

### Browser extension
**Что:** Chrome/Firefox extension с одной кнопкой "Save to Mnemonic" на chatgpt.com / claude.ai / cursor.com — клик подписывает текущий чат через WASM core, отправляет на hosted MCP.
**Риски:** ShadowPrompt-class XSS (Anthropic's own extension shipped a critical zero-click prompt-injection vuln в марте 2026). Maintenance burden — UI каждой target меняется, парсер ломается. Permission grant friction ("read & change all data").
**Митигация:** Manifest v3 + strict CSP + origin allowlist locked + open-source + audited. Defence in depth — даже если extension compromised, артефакты COSE-signed user keypair (живёт в WASM/Turnkey).
**Эффорт:** ~5-7 dev-days + Chrome Store review (1-3 weeks).

### ChatGPT Apps SDK submission
**Что:** Submit Mnemonic как ChatGPT App для inclusion в ChatGPT Plus/Pro/Free через App Directory.
**Зачем:** ChatGPT Plus/Pro юзеры не могут ставить custom MCP — Apps SDK единственный путь. Reach на 100M+ ChatGPT юзеров.
**Риски:** Review time не публичен (community: weeks-to-months). Crypto/Solana surface может вызвать pushback от reviewer'ов. Position как "verifiable knowledge memory" — не "blockchain protocol".
**Эффорт:** ~3 dev-days submission + indeterminate review wait.

### `.mcpb` desktop bundle для Claude Desktop
**Что:** Zip с `manifest.json` + локальный MCP server, пользователь двойным кликом устанавливает в Claude Desktop.
**Зачем:** One-click install (real one-click, не "edit config JSON yourself") для Claude Desktop юзеров, не зависящих от hosted server.
**Эффорт:** ~1.5 dev-days.

### Mobile share-sheet integrations
Native iOS/Android приложения когда ChatGPT/Claude mobile добавят MCP support. Не GA на 2026-04 — wait-and-see.

### `web+mnemonic://` protocol handler
Через `Navigator.registerProtocolHandler` — webapp регистрирует `web+mnemonic://` scheme, открывает bundle URLs в Mnemonic вместо stale-link experience. Marginal value vs. existing landing page. Skip until volume.

## Out of Scope (won't do)

- Multi-tenant SQLite со встроенными accounts (предпочитаем Turnkey + Arweave permanence модель)
- Email/password auth (если делаем — только через Turnkey)
- Mobile-native SDKs (iOS/Android Rust bindings)
- Python/Go bindings (`pyo3`, `cgo` wrappers) — отдельная инициатива, не часть integration story
- Embedding model marketplace (юзер выбирает свой embedder) — выходит за scope identity/integration
