---
created: 2026-04-26
status: approved
type: feature
size: L
---

# User Spec: mnemonic-integrations (Phase 1 — Cross-Provider Memory via MCP)

## Что делаем

Превращаем Mnemonic из локального протокола в **доступный сервис cross-provider памяти для AI-агентов**. Ключевая идея: интеграция = установка Mnemonic MCP в нужный AI-tool. После установки модель в Tool A (например, ChatGPT) вызывает `mnemonic_sign_memory` на естественноязыковую команду пользователя ("save findings onchain"), модель в Tool B (например, Claude.ai) вызывает `mnemonic_recall` и получает атестованный контекст. Всё через MCP-протокол, без скрейпинга, копи-паста или DOM-инъекций.

Phase 1 шипает три delivery-канала:
1. **MCP сервер** — hosted `mcp.mnemonik.xyz` (streamable HTTP + OAuth 2.1 + PKCE) для browser-only юзеров, плюс docker-образ для self-host и листинг в Smithery для discovery.
2. **WASM webapp** на `mnemonik.xyz` — onboarding + install-хаб. Базируется на `mnemonic-core` через wasm-pack.
3. **Marketing site** — landing-страница на том же `mnemonik.xyz`, объясняет протокол, ведёт на install-хаб.

Phase 1 (хакатонный MVP) использует **localStorage Ed25519 keypair** (генерится в браузере через WASM core), JSON-export/import для cross-device. **Turnkey MPC** (email-onboarding + recovery) — Phase 1.5, заменит localStorage без изменения user identity (тот же pubkey мигрирует в Turnkey custody). Self-host docker всегда работает с локальным keypair-файлом.

## Зачем

Сегодня память AI-агента живёт внутри одного провайдера и теряется при switch'е. Юзер делает research в Cursor — продолжить в Claude.ai невозможно без ручного re-explanation. Mnemonic уже решает persistence + verifiability на уровне core, но без hosted MCP протокол доступен только тем, кто умеет собирать docker и настраивать stdio. **Hosted MCP + Turnkey onboarding = протокол доступен любому юзеру с email** и работает в любом MCP-capable AI tool. Это тот скачок UX, без которого "verifiable persistent memory" остаётся академическим тезисом.

Дополнительно: MCP стал de facto стандартом handoff в 2026 (см. `research.md`). Окно стать первым атестованным MCP-сервером в каталогах открыто сейчас. Phase 1 ship'ается под презентацию на хакатоне — успех = working demo + первые внешние юзеры/разработчики, не production-grade scale.

## Как должно работать

### Сценарий 1 — Onboarding (хакатонный MVP)

Юзер открывает `mnemonik.xyz`, кликает "Get started". WASM core генерит Ed25519 keypair прямо в браузере, сохраняет в localStorage (encrypted with passphrase или as-is для MVP). Webapp показывает identity: DID/pubkey + кнопку "Download keypair backup" (JSON-файл). Cross-device — юзер импортирует backup на втором устройстве через "Restore from backup" (Phase 1.5 заменит на Turnkey email-recovery без изменения pubkey).

### Сценарий 2 — Установка Mnemonic в Tool A (Cursor)

На webapp юзер кликает "Install in Cursor" → deeplink `cursor://anysphere.cursor-deeplink/mcp/install?...` открывает Cursor → коннектор `mcp.mnemonik.xyz` устанавливается → запускается OAuth 2.1 PKCE flow → юзер на webapp подтверждает "Allow Cursor to access your Mnemonic" → WASM core подписывает OAuth challenge localStorage-ключом → JWT токен в Cursor. Идентично для VS Code, Claude.ai (через Settings → Connectors → Add custom), Perplexity Pro.

### Сценарий 3 — sign_memory в Tool A

Юзер в Cursor говорит модели "Save my blockchain research findings onchain". Cursor (модель) интерпретирует, вызывает `mnemonic_sign_memory(content="<summary>", tags=["blockchain"])` (юзер видит approval-prompt от Cursor, подтверждает). Hosted MCP: embed → COSE sign server-side identity → Arweave + Solana memo → запись в attestations DB scope-нутую по user pubkey (из JWT). Возвращает `attestation_id`, Arweave URL, Solana tx. Юзер платит через existing `payment.rs` (`PAYMENT_MODE=balance`); хакатон-MVP может стартовать с `PAYMENT_MODE=none` (free) и включить billing post-хакатон.

### Сценарий 4 — recall в Tool B

Юзер открывает свежий чат в Claude.ai (с уже установленным Mnemonic коннектором). Говорит "recall my recent blockchain research". Claude вызывает `mnemonic_recall(query="recent blockchain research", limit=5)`. Hosted MCP: cosine search по user's attestations → top-K с подписью → Claude видит контент, продолжает работу. Recall бесплатный.

### Сценарий 5 — Self-host через Docker (backlog)

Self-host docker image публикация в GHCR — backlog (см. `backlog.md`). Существующий локальный `cargo run -p mnemonic-mcp` self-host работает как сейчас, без OAuth, с локальным keypair-файлом и stdio transport.

### Сценарий 6 — Discovery через Smithery

Агент (например, Claude Code в headless режиме) ищет `mnemonic` в Smithery → находит листинг → показывает install-deeplink → агент сам устанавливает MCP коннектор себе.

## Критерии приёмки

**MUST для Phase 1 (hackathon MVP, ≤2 недели, ~11 dev-days):**

- [ ] `mcp.mnemonik.xyz` отвечает на `tools/list` через streamable HTTP (per MCP spec 2025), доступен публично
- [ ] OAuth 2.1 + PKCE endpoints (`/oauth/authorize`, `/oauth/token`) работают; JWT токен issued bound к user pubkey
- [ ] WASM core (через wasm-bindgen + wasm-pack) экспортирует `generate_keypair`, `sign_challenge`, `export_keypair_json`, `import_keypair_json`; webapp использует это для in-browser identity в localStorage; OAuth challenge подписывается этим ключом
- [ ] Webapp `mnemonik.xyz` имеет 2 страницы: (1) landing с объяснением протокола, (2) install-hub с deeplinks для Cursor / VS Code / Claude.ai + identity-блок (показ DID/pubkey, export/import keypair)
- [ ] **`STORAGE_MODE=local`** для хакатон-демо: SQLite-only, синтетические `local:` ID, без Arweave/Solana RPC вызовов. Демо работает offline на сцене.
- [ ] `smithery.yaml` в репо, листинг на smithery.ai активен с install-deeplink на `mcp.mnemonik.xyz`
- [ ] CI: MCP Inspector валидирует JSON-RPC + tool descriptions на каждый PR; pre-release smoke: ручной чек-лист (Cursor + Claude.ai прогон), документированный в `tasks/`
- [ ] `cargo test --workspace` зелёный, `cargo clippy --workspace --all-targets -- -D warnings` без предупреждений
- [ ] Backward-compat: stdio transport работает как сейчас; existing 5 MCP tools сохраняют JSON-RPC сигнатуры
- [ ] `payment.rs` НЕ рефакторится: `PAYMENT_MODE=none` для хакатона; pubkey-as-user-identity hook готовится без активации billing
- [ ] `core/` не имеет references на OAuth/HTTP transport — изоляция архитектуры сохранена
- [ ] Round-trip: COSE-signed CBOR проходит через mock MCP-прокси байт-в-байт без re-encoding (test в CI)

**Backlog (всё post-hackathon)** — см. `work/mnemonic-integrations/backlog.md` для полного списка: Turnkey MPC integration, Docker GHCR publish, npm `@mnemonic/core`, browser extension, дополнительные webapp страницы (bundles browser / top-up / privacy / stats), `PAYMENT_MODE=balance`, headless Claude Code CI, Anthropic Connectors / mcp.directory / Glama listings, full `STORAGE_MODE=full` (Arweave + Solana) на демо.

**Success metrics для хакатон-демо:**

- [ ] Demo на сцене: один live прогон onboarding → install в Cursor → sign_memory → перейти в Claude.ai → recall — без сбоев
- [ ] ≥3 unique signup'а во время или после презентации (не считая команды)
- [ ] ≥1 внешний разработчик ставит MCP через Smithery deeplink самостоятельно
- [ ] Webapp uptime во время демо-окна (часовое окно): 100%

## Ограничения

- **Phase 1 = хакатон-MVP, ≤2 недели, ~11 dev-days.** Всё что не in MUST → `backlog.md`.
- **Storage на демо = `STORAGE_MODE=local`** (SQLite-only, синтетические IDs). Arweave/Solana код в `core/` не трогаем — он остаётся работоспособным, но не вызывается на сцене. Полная on-chain атестация в demo — backlog.
- **Identity = WASM-генерируемый Ed25519 keypair в браузере + localStorage**. JSON export/import для cross-device. **Turnkey MPC — backlog**: тот же pubkey мигрирует в Turnkey custody без смены DID.
- **`payment.rs` не рефакторится**. `PAYMENT_MODE=none` для хакатон-демо. Pubkey-as-user-identity hook готовится без активации billing.
- **`core/` не модифицируется по бизнес-логике** — HTTP/OAuth/Turnkey код живёт только в `mcp/`. Единственное допустимое изменение в `core/` — добавление `#[wasm_bindgen]` обёрток над existing identity-функциями (gated `#[cfg(target_arch = "wasm32")]`).
- **Self-host текущий — без изменений**. Docker GHCR publish — backlog.
- **WASM используется только в webapp** — не в hosted MCP server.
- **Один реестр в Phase 1** — Smithery. Остальное (Anthropic Connectors, mcp.directory, Glama, Apps SDK) — backlog.
- **Webapp в Phase 1 — 2 страницы** (landing + install-hub с identity-блоком). Bundles browser, top-up, privacy, stats — backlog.
- **Browser extension — backlog.**

## Риски

- **R1: COSE подпись invalidates через Anthropic/OpenAI MCP прокси.** Streamable HTTP коннектор-прокси могут re-encode CBOR байты при passthrough → подпись становится invalid. Митигация: round-trip тест в CI (mock Anthropic-прокси). Fallback: передавать bundle как base64-строку в JSON-поле, парсить на клиенте.
- **R2: localStorage keypair loss = identity loss.** Юзер чистит браузер / меняет устройство без backup → теряет доступ к своим attestations навсегда (на хакатоне это допустимо). Митигация: настойчивый prompt "Download your keypair backup" сразу после генерации; warning при выходе со страницы без backup. Permanent fix — Turnkey в Phase 1.5.
- **R3: IP allowlist на хостинге блокирует Anthropic/OpenAI.** Anthropic публикует IP-диапазоны для connector traffic. Митигация: deploy на Cloudflare/Fly.io с явной WAF-настройкой; e2e-проверка из known Anthropic egress IP до демо.
- **R4: Crypto/Solana surface area триггерит отказ в листинге.** Smithery — community-driven, низкий риск. Anthropic Connectors — высокий (review-time + crypto framing). Митигация: позиционировать как "verifiable knowledge memory", лидировать utility, blockchain — plumbing.
- **R5: ChatGPT Plus tier MCP gating.** OpenAI: full MCP только для Business/Enterprise/Edu. Plus/Pro могут юзать только admin-published Apps SDK apps. Митигация: явное предупреждение на webapp ("ChatGPT Plus не поддерживает custom MCP, используй Cursor/Claude.ai"); Apps SDK submission — отдельный track post-хакатон.
- **R6: CursorJack-class deeplink phishing.** Cursor может добавить trust-prompts, превращая 1-клик в 2-клик. Митигация: deeplink хостится только с verified `mnemonik.xyz` HTTPS; готовы к UI-changes на стороне Cursor.
- **R7: Live-demo failure на сцене.** На хакатоне сеть может быть нестабильной, OAuth flow может зависнуть. Митигация: pre-recorded fallback-видео; локальный docker self-host как backup-демо без зависимости от hosted-сервиса.

## Технические решения

- **Transport:** streamable HTTP (не SSE — phasing out, см. research.md). Stdio сохраняется для self-host.
- **Auth:** провайдер-agnostic OAuth 2.1 + PKCE — поднимаем собственный OAuth server (никакой Auth0/Clerk lock-in).
- **Wallet/Identity (хакатон-MVP):** WASM core генерит Ed25519 keypair в браузере (через `core/src/identity` функции, exposed через wasm-bindgen). Хранение — localStorage (raw JSON или encrypted with passphrase, выбор в tech-spec). Export/import — JSON-файл. JWT issued против user pubkey. **P1.5 миграция в Turnkey MPC** — тот же pubkey мигрирует в Turnkey custody, существующие attestations остаются валидными.
- **Storage:** Arweave для attestations (existing `core/src/arweave.rs`), Solana memo для timestamp anchor (existing `core/src/solana.rs`). Никакой multi-tenant SQLite — bundle "owned by" pubkey, lookup по pubkey + cosine search в hosted-server attestations.db.
- **Webapp framework:** существующий React+Vite+Tailwind в `webapp/` (см. architecture.md). Новые routes в Phase 1: `/` (landing), `/install` (install-hub + identity inline). WASM core импортируется как сейчас.
- **Hosting:** TBD в tech-spec. Existing VPS `mnemonik.xyz` (см. deployment.md) — кандидат №1 (нет vendor lock, low cost). Fly.io / Cloudflare Workers — alternative для better latency. Требование: public reachability, TLS, IP-доступ для Anthropic/OpenAI connector ranges.
- **Tests:** MCP Inspector в GitHub Action на каждый PR (валидирует tool schemas + JSON-RPC). Pre-release smoke — ручной чек-лист (Cursor + Claude.ai прогон). Headless Claude Code в CI — Phase 1.5.
- **Observability:** existing `tracing` в `mcp/`. Дополнительно — counter для install events (success metrics), для sign_memory/recall calls (хакатон demo monitoring).

## Тестирование

**Unit:** OAuth flow (mock client), HTTP transport, JWT issue/validate, WASM keypair gen + sign challenge, deeplink URL builder, COSE round-trip helpers.

**Integration:** httpmock для Anthropic/OpenAI MCP прокси-passthrough; round-trip COSE-validation после passthrough.

**E2E (CI):** MCP Inspector прогон tools/list + tools/call для всех 5 инструментов на каждом PR.

**E2E (manual, pre-demo):** документированный чек-лист в `tasks/` — на реальных аккаунтах прогнать onboarding → install в Cursor → sign_memory → переход в Claude.ai → recall. Цель — поймать UX-фрикшены до демо.

**Headless Claude Code в CI** — Phase 1.5.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|---------------------|
| 1. Hosted MCP отвечает | `bash: curl -X POST https://mcp.mnemonik.xyz -d '{"method":"tools/list"}'` | 5 tools returned |
| 2. OAuth flow работает | `bash: scripts/test-oauth-flow.sh` | JWT issued, payload содержит user pubkey |
| 3. Tests зелёные | `bash: cargo test --workspace --no-fail-fast` | Все тесты, включая httpmock proxy |
| 4. Clippy чистый | `bash: cargo clippy --workspace --all-targets -- -D warnings` | Ноль warnings |
| 5. STORAGE_MODE=local на демо | `bash: curl -X POST https://mcp.mnemonik.xyz/mcp -d '{"method":"tools/call","params":{"name":"mnemonic_sign_memory","arguments":{"content":"test"}}}'` | attestation_id с префиксом `local:`, без Arweave/Solana вызовов |
| 6. Smithery листинг | `bash: curl -fsSL https://smithery.ai/mcp/mnemonic` | 200 OK, install-deeplink в HTML |
| 7. core/ не тронут OAuth/HTTP | `bash: grep -rE "OAuth\|http_transport\|axum" core/src/` | Пустой результат |
| 8. payment.rs неизменён по схеме | `bash: git diff main -- mcp/src/payment.rs \| grep -E "^-CREATE TABLE\|^-ALTER TABLE"` | Пустой результат |
| 9. MCP Inspector | `bash: npx @modelcontextprotocol/inspector --validate https://mcp.mnemonik.xyz` | All checks pass |
| 10. Round-trip COSE через прокси | `bash: cargo test -p mnemonic-mcp roundtrip_cose_via_http_proxy` | Подпись валидна после passthrough |
| 11. Webapp routes | `bash: for r in / /install; do curl -fI https://mnemonik.xyz$r; done` | Оба возвращают 200 |

### Пользователь проверяет

- На свежем браузере без cookies: открыть `mnemonik.xyz`, кликнуть "Get started" → WASM генерит keypair → видишь свой DID/pubkey + кнопку "Download backup". Скачать backup-JSON, проверить что содержит pubkey + private bytes.
- На Cursor (свежий профиль): кликнуть "Install in Cursor" на webapp → deeplink → OAuth approve → в новом чате сказать "save this onchain: hello world from cursor" → модель вызовет sign_memory с user approval → получить attestation_id, Arweave URL. Открыть Arweave URL — видеть содержимое.
- На Claude.ai Pro: добавить custom connector `mcp.mnemonik.xyz` через Settings → авторизоваться тем же pubkey → в свежем чате сказать "recall my recent saves" → модель вызовет recall → увидеть "hello world from cursor" с COSE-подписью.
- На втором устройстве: импортировать backup-JSON в webapp → войти под тем же DID → cursor coupling работает идентично.
- Self-host (текущее, без изменений): `cargo run -p mnemonic-mcp -- --transport stdio` локально, подключить к локальному Cursor — работает как сейчас, без OAuth, с локальным keypair-файлом. Docker GHCR publish — backlog.
