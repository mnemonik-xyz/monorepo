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
1. **MCP сервер** — hosted `mcp.mnemonic.dev` (streamable HTTP + OAuth 2.1 + PKCE) для browser-only юзеров, плюс docker-образ для self-host и листинг в Smithery для discovery.
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

На webapp юзер кликает "Install in Cursor" → deeplink `cursor://anysphere.cursor-deeplink/mcp/install?...` открывает Cursor → коннектор `mcp.mnemonic.dev` устанавливается → запускается OAuth 2.1 PKCE flow → юзер на webapp подтверждает "Allow Cursor to access your Mnemonic" → WASM core подписывает OAuth challenge localStorage-ключом → JWT токен в Cursor. Идентично для VS Code, Claude.ai (через Settings → Connectors → Add custom), Perplexity Pro.

### Сценарий 3 — sign_memory в Tool A

Юзер в Cursor говорит модели "Save my blockchain research findings onchain". Cursor (модель) интерпретирует, вызывает `mnemonic_sign_memory(content="<summary>", tags=["blockchain"])` (юзер видит approval-prompt от Cursor, подтверждает). Hosted MCP: embed → COSE sign server-side identity → Arweave + Solana memo → запись в attestations DB scope-нутую по user pubkey (из JWT). Возвращает `attestation_id`, Arweave URL, Solana tx. Юзер платит через existing `payment.rs` (`PAYMENT_MODE=balance`); хакатон-MVP может стартовать с `PAYMENT_MODE=none` (free) и включить billing post-хакатон.

### Сценарий 4 — recall в Tool B

Юзер открывает свежий чат в Claude.ai (с уже установленным Mnemonic коннектором). Говорит "recall my recent blockchain research". Claude вызывает `mnemonic_recall(query="recent blockchain research", limit=5)`. Hosted MCP: cosine search по user's attestations → top-K с подписью → Claude видит контент, продолжает работу. Recall бесплатный.

### Сценарий 5 — Self-host через Docker

Технический юзер делает `docker pull ghcr.io/mnemonik-xyz/mnemonic-mcp:latest`, запускает с локальным keypair (`MNEMONIC_KEYPAIR_PATH=...`), без Turnkey, без OAuth — текущий stdio-режим работает как сейчас. Опционально юзер настраивает свой Cursor через `mcpb` или ручную stdio-конфигурацию.

### Сценарий 6 — Discovery через Smithery

Агент (например, Claude Code в headless режиме) ищет `mnemonic` в Smithery → находит листинг → показывает install-deeplink → агент сам устанавливает MCP коннектор себе.

## Критерии приёмки

**MUST для Phase 1 (hackathon MVP):**

- [ ] `mcp.mnemonic.dev` отвечает на `tools/list` через streamable HTTP, доступен из IP-диапазонов Anthropic и OpenAI
- [ ] OAuth 2.1 + PKCE endpoints (`/oauth/authorize`, `/oauth/token`) работают; JWT токен issued bound к user pubkey
- [ ] WASM core генерит Ed25519 keypair в браузере, хранит в localStorage; export/import через JSON-файл; OAuth challenge подписывается этим ключом
- [ ] Webapp `mnemonik.xyz` имеет 2 страницы (MUST): (1) landing с объяснением протокола, (2) install-hub с deeplinks для Cursor / VS Code / Claude.ai + identity-блок (показ DID/pubkey, export/import keypair)
- [ ] Docker image `ghcr.io/mnemonik-xyz/mnemonic-mcp:latest` собирается в CI на git tag, поддерживает stdio + http transports
- [ ] `smithery.yaml` в репо, листинг на smithery.ai активен, install-deeplink работает
- [ ] CI: MCP Inspector валидирует JSON-RPC + tool descriptions на каждый PR; pre-release smoke: ручной прогон `whoami → sign_memory → recall` через Cursor с auth, документированный в `tasks/`
- [ ] `cargo test --workspace` зелёный, `cargo clippy --workspace --all-targets -- -D warnings` без предупреждений
- [ ] Backward-compat: stdio transport работает как сейчас; existing 5 MCP tools сохраняют JSON-RPC сигнатуры
- [ ] `payment.rs` НЕ рефакторится: для хакатона `PAYMENT_MODE=none` (free demo); хук на pubkey-as-user-identity для billing post-хакатон
- [ ] `core/` не имеет references на OAuth/HTTP transport — изоляция архитектуры сохранена
- [ ] Round-trip: COSE-signed CBOR проходит через Anthropic/OpenAI MCP прокси байт-в-байт без re-encoding (тест в CI)

**SHOULD для Phase 1.5 (post-hackathon):**

- [ ] **Turnkey MPC integration** — заменяет localStorage keypair на email-onboarded MPC-wallet с recovery; existing pubkey мигрирует в Turnkey custody без потери identity
- [ ] Webapp дополнительные страницы: (3) bundles browser (список attestations + Arweave/Solana ссылки), (4) top-up balance UI, (5) bundle delete/privacy controls, (6) stats dashboard
- [ ] `PAYMENT_MODE=balance` включён, billing работает per-call
- [ ] Headless Claude Code прогон в CI (nightly + pre-release)
- [ ] Листинг в Anthropic Connectors Directory (partner outreach), mcp.directory, Glama
- [ ] `@mnemonic/core` опубликован на npm

**Success metrics для хакатон-демо:**

- [ ] Demo на сцене: один live прогон onboarding → install в Cursor → sign_memory → перейти в Claude.ai → recall — без сбоев
- [ ] ≥3 unique signup'а во время или после презентации (не считая команды)
- [ ] ≥1 внешний разработчик ставит MCP через Smithery deeplink самостоятельно
- [ ] Webapp uptime во время демо-окна (часовое окно): 100%

## Ограничения

- **Только Phase 1 (хакатонный MVP)**. Browser extension, Apps SDK submission, `.mcpb` для Claude Desktop, mobile share-sheet, Gemini CLI extension — отдельные итерации.
- **Turnkey MPC — Phase 1.5**. Хакатон-MVP использует localStorage keypair с JSON export/import. Миграция в Turnkey custody — без смены user pubkey (тот же DID).
- **`payment.rs` не рефакторится**. Для хакатона `PAYMENT_MODE=none` (бесплатное демо). Pubkey-as-user-identity hook готовится без активации billing-логики до post-хакатон.
- **`core/` не модифицируется** — HTTP/OAuth код живёт только в `mcp/`. Архитектурное правило (см. CLAUDE.md) нерушимо.
- **Self-host docker остаётся с локальным keypair** — никакого OAuth для self-host пути, текущий stdio-режим работает как сейчас.
- **WASM используется только в webapp** — не в hosted MCP.
- **Storage** — Arweave для attestations, localStorage только для keypair и UI-state. Никаких user attestations в localStorage — теряется durability.
- **Один реестр в Phase 1** — Smithery. Anthropic Connectors / mcp.directory / Glama — параллельный outreach без deadline, реализация в P1.5+.
- **Webapp в Phase 1 — 2 страницы** (landing + install-hub с identity-блоком). Bundles browser, top-up, privacy, stats — Phase 1.5.

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
| 1. Hosted MCP отвечает | `bash: curl -X POST https://mcp.mnemonic.dev -d '{"method":"tools/list"}'` | 5 tools returned |
| 2. OAuth flow работает | `bash: scripts/test-oauth-flow.sh` | JWT issued, payload содержит user pubkey |
| 3. Tests зелёные | `bash: cargo test --workspace --no-fail-fast` | Все тесты, включая httpmock proxy |
| 4. Clippy чистый | `bash: cargo clippy --workspace --all-targets -- -D warnings` | Ноль warnings |
| 5. Docker image | `bash: docker pull ghcr.io/mnemonik-xyz/mnemonic-mcp:latest && docker run --rm ghcr.io/mnemonik-xyz/mnemonic-mcp:latest --version` | Тег latest присутствует, бинарь запускается |
| 6. Smithery листинг | `bash: curl -fsSL https://smithery.ai/mcp/mnemonic` | 200 OK, install-deeplink в HTML |
| 7. core/ не тронут OAuth/HTTP | `bash: grep -rE "OAuth\|http_transport\|axum" core/src/` | Пустой результат |
| 8. payment.rs неизменён по схеме | `bash: git diff main -- mcp/src/payment.rs \| grep -E "^-CREATE TABLE\|^-ALTER TABLE"` | Пустой результат |
| 9. MCP Inspector | `bash: npx @modelcontextprotocol/inspector --validate https://mcp.mnemonic.dev` | All checks pass |
| 10. Round-trip COSE через прокси | `bash: cargo test -p mnemonic-mcp roundtrip_cose_via_http_proxy` | Подпись валидна после passthrough |
| 11. Webapp routes | `bash: for r in / /install; do curl -fI https://mnemonik.xyz$r; done` | Оба возвращают 200 |

### Пользователь проверяет

- На свежем браузере без cookies: открыть `mnemonik.xyz`, кликнуть "Get started" → WASM генерит keypair → видишь свой DID/pubkey + кнопку "Download backup". Скачать backup-JSON, проверить что содержит pubkey + private bytes.
- На Cursor (свежий профиль): кликнуть "Install in Cursor" на webapp → deeplink → OAuth approve → в новом чате сказать "save this onchain: hello world from cursor" → модель вызовет sign_memory с user approval → получить attestation_id, Arweave URL. Открыть Arweave URL — видеть содержимое.
- На Claude.ai Pro: добавить custom connector `mcp.mnemonic.dev` через Settings → авторизоваться тем же pubkey → в свежем чате сказать "recall my recent saves" → модель вызовет recall → увидеть "hello world from cursor" с COSE-подписью.
- На втором устройстве: импортировать backup-JSON в webapp → войти под тем же DID → cursor coupling работает идентично.
- Self-host: `docker pull ghcr.io/mnemonik-xyz/mnemonic-mcp:latest`, запустить локально с stdio, подключить к локальному Cursor — работает как сейчас, без OAuth, с локальным keypair.
