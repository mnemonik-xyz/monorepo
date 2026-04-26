---
created: 2026-04-26
status: draft
type: feature
size: L
---

# User Spec: mnemonic-integrations (AI Tools Integration, Phase 1)

## Что делаем

Доставляем атестованные context bundles напрямую в свежий чат внешнего AI-приложения (ChatGPT, Claude.ai, Cursor, VS Code, Perplexity и др.) без требования скачивать файл. Реализуем три механизма доставки одной фичей: (1) hosted remote MCP server `mcp.mnemonic.dev` со streamable HTTP + OAuth 2.1; (2) per-bundle landing page `mnemonic.dev/b/<id>` с deeplink-кнопками для каждого таргета; (3) "Copy as prompt" fallback с self-contained markdown шаблоном, ссылающимся на Arweave + COSE-подпись.

Scope — **Phase 1** из `research.md`. Apps SDK submission, `.mcpb`, browser extension — отдельные итерации.

## Зачем

Сейчас единственный способ передать атестованный bundle в другой AI tool — скачать файл и приложить вручную. Это блокирует ключевой UX мнемоника: "verifiable knowledge memory" должна стартовать в один клик из любого AI-приложения. Без этого протокол выглядит академически — пользователь не может сравнить ChatGPT-ответ с/без атестованного контекста, и не видит ценности подписи. MCP стал de facto стандартом handoff в 2026 — окно, чтобы стать первым атестованным MCP-сервером в каталогах, открыто сейчас.

## Как должно работать

### Сценарий 1 — Cursor / VS Code разработчик (one-click)

Пользователь открывает `mnemonic.dev/b/<bundle-id>` (например, перейдя с маркетинг-страницы). Нажимает "Open in Cursor" — Cursor открывается через `cursor://anysphere.cursor-deeplink/mcp/install?...`, MCP-коннектор `mcp.mnemonic.dev` устанавливается, запускается OAuth, в свежем чате `mnemonic_recall(bundle_id)` возвращает COSE-подписанный CBOR. Идентично для VS Code через `vscode:mcp/install`.

### Сценарий 2 — Claude.ai Pro/Max (3 клика)

Пользователь на landing page нажимает "Add to Claude.ai" — открывается модалка с URL `mcp.mnemonic.dev` и одной строкой инструкции (Settings → Connectors → Add custom connector). После OAuth `mnemonic_verify` и `mnemonic_recall` доступны как tool calls в любом чате.

### Сценарий 3 — ChatGPT Plus (clipboard fallback)

Plus-юзер не может ставить custom MCP. Кнопка "Open in ChatGPT" копирует self-contained markdown в clipboard и открывает `chatgpt.com/?q=<short-prompt>` в новой вкладке. Шаблон содержит: краткое summary, Arweave URL, COSE-подпись base64, инструкцию "если есть verify tool — вызови, иначе доверяй по reference на Arweave". Длина ≤ 6000 символов.

### Сценарий 4 — Perplexity Pro / Windsurf / Zed

Кнопка "Copy MCP URL" с инструкцией для paste в коннекторы — единый путь.

## Критерии приёмки

- [ ] `mnemonic-mcp` слушает streamable HTTP transport (`--transport http`) с OAuth 2.1 + PKCE; стdio-режим продолжает работать
- [ ] `cargo test -p mnemonic-mcp --features http-oauth` — зелёный, включая mock OAuth flow
- [ ] Хост `mcp.mnemonic.dev` отвечает на `tools/list` за <500ms из IP-диапазонов Anthropic и OpenAI (проверяется через external probe)
- [ ] Веб-страница `mnemonic.dev/b/<bundle-id>` рендерит верификацию (signature ✓, Solana tx ✓, Arweave URL) и 5 кнопок: Cursor, VS Code, Claude.ai, ChatGPT, Copy as prompt
- [ ] Cursor deeplink проверен ручным тестом — установка коннектора в один клик из landing page
- [ ] VS Code deeplink проверен ручным тестом — `vscode:mcp/install` устанавливает коннектор
- [ ] Claude.ai modal flow проверен на Pro-аккаунте — `mnemonic_recall` доступен в свежем чате
- [ ] ChatGPT Plus copy-as-prompt проверен — markdown ≤ 6000 символов, `?q=` корректно префиллит ввод
- [ ] Self-contained markdown template включает Arweave URL и base64 COSE-подпись; модель в свежем чате может re-verify через built-in URL fetch
- [ ] Текущая download-кнопка на маркетинг-сайте заменена на "Get protocol knowledge" → ведёт на landing page; "Download raw bundle" остаётся как третичная опция
- [ ] `architecture.md` содержит описание HTTP/OAuth слоя в `mcp/`
- [ ] Round-trip: bundle, переданный через MCP, проходит COSE-валидацию байт-в-байт (no re-encoding via streamable HTTP proxy)

## Ограничения

- **Только Phase 1** из research. Apps SDK submission, `.mcpb`, browser extension, mobile share-sheet, Gemini CLI extension, Smithery listing — вне scope.
- **Auth**: OAuth 2.1 с PKCE; собственный OAuth provider или Auth0/Clerk — решается в tech-spec, но без рефакторинга existing payment-логики (`mcp/src/payment.rs` остаётся как есть).
- **MCP API совместимость**: 5 текущих инструментов (`whoami`, `sign_memory`, `verify`, `prove_identity`, `recall`) сохраняют JSON-RPC сигнатуры. Возможно добавление одного нового — `get_protocol_knowledge` для маркетинг-flow.
- **Storage modes**: hosted MCP работает в `full` mode (Arweave + Solana). `local` mode остаётся для self-hosted, через ту же бинарь.
- **Архитектурные правила**: HTTP/OAuth слой живёт в `mcp/`, никаких дополнений в `core/`. `pricing.rs` и payment-методы не мигрируют.
- **Никаких WASM** — webapp использует hosted MCP API, не in-browser core.
- **Claude.ai `?q=`** не используем — был удалён в Oct 2025.

## Риски

- **Риск 1: COSE-подпись invalidates через streamable HTTP proxy.** Anthropic/OpenAI коннектор-прокси могут re-encode CBOR. Митигация: round-trip-тест через прокси в CI; если ломается — передавать bundle как base64-строку в JSON-поле, дешифровать на клиенте.
- **Риск 2: IP allowlist на хостинге блокирует Anthropic/OpenAI.** Митигация: deploy на Cloudflare/Fly.io с явной настройкой WAF; тест из known Anthropic egress IP до GA.
- **Риск 3: CursorJack-style deeplink phishing.** Cursor может добавить trust-prompts, превращая 1-клик в 2-клик. Митигация: deeplink хостится только с verified `mnemonic.dev` HTTPS; landing page подписана через CT log.
- **Риск 4: ChatGPT `?q=` deprecation.** OpenAI может убрать вслед за Claude.ai. Митигация: copy-to-clipboard работает независимо от `?q=`; кнопка "Open ChatGPT" graceful degrade на просто "open in new tab".
- **Риск 5: OAuth complexity blocks free-tier users.** Claude.ai Free даёт один connector slot. Митигация: чёткое сообщение в modal про лимит; clipboard-путь как универсальный fallback.

## Технические решения

- **Transport**: streamable HTTP (не SSE — phasing out). MCP Rust SDK поддерживает.
- **OAuth provider**: TBD в tech-spec — рассмотреть собственный (Ed25519 identity → JWT) vs. Auth0 vs. Clerk.
- **Hosting**: TBD в tech-spec — Fly.io / Cloudflare Workers / собственный VPS. Требование: public reachability + low latency для tool calls.
- **Landing page**: рендерится из существующего webapp (`mnemonic-webapp`), per-bundle SSR через bundle-id route.
- **Markdown template**: генерируется server-side из bundle metadata; embed Arweave URL + truncated base64 COSE-подпись + instructional preamble.
- **Deeplink generation**: статические URL-encoded конфиги для Cursor/VS Code, генерируется на landing page client-side.

## Тестирование

**Unit-тесты:** OAuth flow (mock provider), HTTP transport (mock client), markdown template generator (snapshot tests), deeplink URL builder.

**Интеграционные тесты:** httpmock для Anthropic/OpenAI MCP-прокси round-trip; round-trip COSE validation после passthrough.

**E2E (manual)**: 5 ручных сценариев на реальных аккаунтах: Cursor, VS Code, Claude.ai Pro, ChatGPT Plus, Perplexity Pro. Чек-лист в `tasks/`.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|---------------------|
| 1. HTTP transport компилируется | `bash: cargo build -p mnemonic-mcp --features http-oauth` | Успешно |
| 2. Тесты зелёные | `bash: cargo test -p mnemonic-mcp` | Все тесты, включая OAuth mock |
| 3. Clippy чистый | `bash: cargo clippy --workspace --all-targets -- -D warnings` | Ноль warnings |
| 4. tools/list через HTTP | `bash: curl -X POST $URL -d '{"method":"tools/list"}'` | Возвращает 5 (или 6) инструментов |
| 5. Round-trip CBOR | `bash: cargo test -p mnemonic-mcp roundtrip_cose_via_http` | Подпись валидна после прокси |
| 6. core/ не тронут payment-логикой | `bash: grep -r "OAuth\|http_transport" core/src/` | Пустой результат |
| 7. Landing page route | `bash: curl mnemonic.dev/b/<id>` | 200 OK, рендерит 5 кнопок |

### Пользователь проверяет

- На свежем Cursor-аккаунте: открыть `mnemonic.dev/b/<demo-bundle>`, нажать "Open in Cursor", пройти OAuth, в новом чате вызвать `recall` — получить контент demo-bundle с валидной подписью.
- На Claude.ai Pro: добавить коннектор через modal, вызвать `verify` в чате — получить подтверждение Solana tx и Arweave URL.
- На ChatGPT Plus: нажать "Open in ChatGPT", вставить, отправить — модель reference-ит Arweave URL в ответе.
- Open `~/.mnemonic/attestations.db` через sqlite3 после всех сценариев — убедиться что hosted MCP корректно пишет attestations (если single-tenant) или per-user (если multi-tenant).
