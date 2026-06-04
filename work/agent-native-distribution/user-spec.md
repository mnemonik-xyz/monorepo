---
created: 2026-06-04
status: draft
type: feature
size: L  # 3 coordinated pieces — server-side propagation + CLI install + CLI mcp-stdio + new visibility column
priority: P0 (без него adoption story не работает: установленный, но не использующийся протокол не distributes)
related:
  - work/invisible-identity/user-spec.md (опирается — local identity bootstrap reused для local-mode signing)
  - work/modes-user-choice/user-spec.md (опирается — write_mode local/participate, AC #156)
  - work/agent-native-distribution/TASK-agent-native-distribution.md (исходный технический бриф)
source_of_patterns: github.com/aitankfish/pnl (@pnlmarket/mcp-server v0.5.1, MIT). Patterns reused (npm-bin-is-installer-and-server, hardcoded host candidates, non-destructive JSON merge, skill-file copying). Code NOT copied. PNL's centralized admin model explicitly REJECTED.
---

# User Spec: Agent-Native Distribution — seamless install + MCP-native skill propagation

## Что делаем

Превращаем установку Mnemonic в **одну npm-команду + одну install-команду**, после которой любой агент (Claude Code, Claude Desktop, Cursor) видит Mnemonic-инструменты с полными инструкциями когда и как их использовать. Базовое использование (local-mode attestations) работает **полностью локально, без сети и без OAuth** — приватные памяти никогда не покидают машину пользователя. Платная и публичная ветка (participate-mode, chain-anchoring) запускается OAuth-loopback'ом только при первой записи такого типа.

Три согласованных куска, доставляются одним релизом:

**Кусок 1 — Server-side skill propagation на `mcp.mnemonik.xyz`.**

1. На сервере появляются **7 markdown skill-манифестов** (`help`, `init`, `recall`, `attest`, `checkpoint`, `verify`, `status`) — единый источник правды per skill. Каждый манифест объясняет когда инструмент применять, какой контекст из диалога собрать, что не делать.
2. Манифесты доставляются через **три MCP-поверхности** одновременно: `prompts/list + prompts/get` (для агентов, использующих именованные prompts как slash-команды), `resources/list + resources/read` (для агентов, загружающих manifests как читаемые ресурсы), и **enriched `tools/list` descriptions** для всех 5 существующих инструментов (Purpose + Trigger sections вшиваются на build-time).
3. **Discovery работает pre-auth.** `initialize`, `prompts/*`, `resources/*`, `tools/list` отвечают без Bearer-токена. Любой агент, подключившийся к `mcp.mnemonik.xyz`, мгновенно видит все skills, не пройдя OAuth. Существующий OAuth-gate остаётся на write-tier tool calls в participate mode.
4. **Anonymous recall** работает без auth над публичной частью пула (см. `visibility` ниже).
5. Схема расширяется одной колонкой: `attestations.visibility TEXT NOT NULL DEFAULT 'private'` (values: `'private'` | `'public'`). Anonymous recall возвращает **только** `visibility='public'` строки. Default `'private'` — privacy-by-default.

**Кусок 2 — CLI `mnemonic install` subcommand (PNL pattern, lifted, not copied).**

6. Существующий `@mnemonik-xyz/cli` обогащается `install` subcommand'ом. Хардкоднутый список из 3 хост-конфигов: `~/.claude.json`, `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS), `~/.cursor/mcp.json`. Только-если-файл-существует, non-destructive JSON merge, идемпотентно. Linux/Windows пути и хосты Cline/Codex/Windsurf — out of scope для v1 (повторяем реальное покрытие PNL, не overstatement из их README).
7. В каждый существующий config пишется одна запись:
   ```json
   "mnemonik": { "command": "npx", "args": ["-y", "@mnemonik-xyz/cli", "mcp-stdio"] }
   ```
   Все остальные ключи и MCP-серверы остаются byte-identical. Re-run заменяет в-месте, дубликатов нет.
8. Output говорит пользователю **перезапустить уже запущенные агенты** (хост hot-reload вне scope, как у PNL). Если хост не запущен — открыть его как обычно.

**Кусок 3 — CLI `mnemonic mcp-stdio` subcommand.**

9. Локальный stdio MCP-сервер, который хост (Claude Code/Desktop/Cursor) спавнит как subprocess. JSON-RPC поверх stdin/stdout.
10. **Dual-route по `write_mode`:**
    - **Local mode (default)**: полное локальное исполнение. Embedder через transformers.js (`Xenova/all-MiniLM-L6-v2`, 384-dim, скачивается on-demand при первом write), TurboQuant compress через WASM-core, canonical CBOR + COSE_Sign1 через SDK, INSERT в `~/.mnemonic/attestations.db` (better-sqlite3, схема портирована из `core/src/storage/sqlite.rs`). **Никаких сетевых вызовов.** Подпись локальным Ed25519 identity (из invisible-bootstrap, PR #154/#157 шипнуты ранее сегодня).
    - **Participate mode**: проксирует JSON-RPC в `mcp.mnemonik.xyz/mcp` через HTTPS. На первый auth-required вызов запускает OAuth-loopback (browser popup, как в существующем `mnemonic login --browser`).
    - **Discovery (`prompts/*`, `resources/*`, `tools/list`)**: всегда проксируется на сервер — один источник правды для манифестов.
11. **Token storage переезжает в OS keychain.** Сегодня `~/.mnemonic/token.json` — plaintext. После v1 — keychain entry `xyz.mnemonik.token / default` (macOS Keychain / Secret Service / DPAPI). Это закрывает аномалию: identity у нас уже в keychain через invisible-bootstrap, токен — нет. `mnemonic logout` удаляет keychain entry.
12. **Soft-fall semantics на сбое embedder'а** (model download fail, ONNX runtime crash):
    - `mode=local + visibility=private` → **fail loud**, никаких publishments. Privacy contract is non-negotiable.
    - `mode=local + visibility=public + network available` → **soft-fall к participate**, OAuth-loopback если нужен, stderr line `[mnemonik] local model unavailable, falling through to participate (this attestation will be PUBLIC and chain-anchored)`.
13. **`mnemonic doctor`** — диагностическая команда (user-facing verification). Проверяет presence манифестов в host configs, ping `mcp.mnemonik.xyz/health`, доступность local embedder + model, sqlite read/write, keychain accessibility. Pass/fail per check + repair hints.

## Зачем

**Сегодня протокол установлен, но не используется.**

Даже после успешной ручной настройки (отредактировал `~/.claude.json`, прошёл OAuth) агент **не дотягивается до инструментов в правильные моменты**: пять tool'ов отдаются с минимальными descriptions, никаких behavioural-инструкций. Агент не знает когда attest'ить (после каких решений, не каких нажатий клавиш), что **не** attest'ить (transient state, scratch work, PII), как собрать payload context из диалога, какие guardrails применять. В результате — протокол установлен, но не работает по cadence'у, который превращает его в habit. Без habit'а memory-provenance standard не распространяется, потому что spread зависит от того, что **агенты аттестуют рефлекторно**, а не от того, что пользователь даёт явные команды.

**Сегодня даже установка непропорционально болезненна.**

Чтобы попробовать Mnemonic нужно: найти docs, вручную добавить config-entry в свой агент, перезапустить агента, пройти OAuth-флоу, **и только потом** агент видит инструменты — без behavioural-инструкций. Каждый из шагов — точка отвала. Пользователь, который сегодня хочет local-mode attestations (не платный, не публичный), всё равно обязан пройти полный OAuth-цикл для **любого** sign'а.

**Цель этой фичи:**

- **Установка в две команды**: `npm install -g @mnemonik-xyz/cli && mnemonic install`. После рестарта (или первого старта) агент видит mnemonik. Никаких ручных правок конфигов.
- **Local-mode usage без OAuth и без сети**: подписать память локально, никаких prompts, никаких сетевых хапов.
- **Anonymous discovery до OAuth**: подключённый агент видит skills + recall публичной части пула без идентичности вообще. Это превращает MCP-endpoint в inbound distribution channel: спред идёт через сеть MCP-подключений сама по себе, не через docs-link.
- **OAuth срабатывает только когда явно нужен** — на первый participate-mode write.

## Целевая аудитория

Любой MCP-capable агент или его оператор. Две когорты:

1. **Anonymous / curious**: подключается к `mcp.mnemonik.xyz` без auth, изучает что протокол даёт через prompts и resources, может попробовать anonymous recall публичной части пула. Решает OAuth-init для записи или просто узнаёт.
2. **Authenticated operator**: установил CLI через `mnemonic install`, ожидает что local-mode писалки работают без вопросов, participate-mode требует OAuth и платёж.

Поддерживаемые хосты в v1: **Claude Code, Claude Desktop (macOS), Cursor**. Linux / Windows пути, Cline / Codex / Windsurf — v1.1+.

## Как должно работать

**Поток 1 — Newcomer install.**

1. Пользователь делает `npm install -g @mnemonik-xyz/cli`.
2. Пользователь делает `mnemonic install`. CLI находит существующие хост-конфиги, пишет в каждый ровно одну `mcpServers.mnemonik` запись, не трогает другие ключи, не создаёт конфиги для незаустановленных хостов. Output: «Mnemonik wired into Claude Code, Cursor. If any of them is already running, please restart it.»
3. Пользователь открывает Claude Code. Хост спавнит `npx -y @mnemonik-xyz/cli mcp-stdio` как subprocess. Subprocess подключается к `mcp.mnemonik.xyz` для discovery, advertise'ит 5 tools + 7 prompts + 7 resources хосту.
4. Хост exposуит `/mnemonik-*` slash-commands и инструменты агенту с полными descriptions.
5. Агент видит инструменты + skill-manifests и реагирует ими в правильные моменты — никаких дополнительных действий от пользователя.

**Поток 2 — First local-mode write.**

1. Агент решает аттестовать (скажем, после committed code change — что описано в `mnemonik-attest` манифесте).
2. Агент вызывает `sign_memory { mode: "local", visibility: "private", content: ... }` через mcp-stdio.
3. mcp-stdio: invisibly обеспечивает identity (через invisible-bootstrap из PR #154/#157, no-op если уже есть), embed через transformers.js (первый вызов качает ~30MB MiniLM model в фоне), TurboQuant compress, canonical CBOR, COSE_Sign1 sign, INSERT в `~/.mnemonic/attestations.db`. Никаких сетевых вызовов.
4. Агент получает подтверждение `{attestation_id, content_hash, write_mode: "local", visibility: "private"}`.
5. Пользователь не видит ничего — silent. Никаких browser popup'ов, никаких prompts.

**Поток 3 — First participate-mode write (public chain-anchored).**

1. Агент решает зааттестовать что-то значимое и предлагает пользователю опубликовать (manifest `mnemonik-attest` требует confirmation от пользователя для public).
2. Пользователь подтверждает. Агент вызывает `sign_memory { mode: "participate", visibility: "public", confirmed: true, content: ... }`.
3. mcp-stdio проксирует на `mcp.mnemonik.xyz`. Сервер видит no Bearer → запускает OAuth-loopback: возвращает sign_url, mcp-stdio открывает браузер, пользователь логинится через webapp.
4. Token cached в OS keychain (`xyz.mnemonik.token`). Запись anchor'ится на Solana + Arweave. Stderr line: `[mnemonik] participate-mode write: <pubkey> <content_hash>`.
5. На follow-up participate-mode write'ы token уже в keychain → OAuth не запускается, до 1h TTL.

**Поток 4 — Anonymous discovery (без CLI).**

1. Разработчик добавляет `https://mcp.mnemonik.xyz/mcp` напрямую в `~/.cursor/mcp.json` (без установки CLI).
2. Cursor подключается без Bearer. Сервер пропускает discovery surface; `prompts/list`, `resources/list`, `tools/list` все возвращаются.
3. Разработчик или его агент пробуют `recall` запрос — сервер фильтрует по `visibility='public'` и возвращает публичную часть пула.
4. Если решает participate, следует skill-manifest `mnemonik-init`, который проводит через `npm install -g @mnemonik-xyz/cli && mnemonic install`.

## Критерии приёмки

**Release gate (должно держаться для shippa — protocol-level, без host-specific behavioural verification):**

- [ ] **AC1 — Anonymous discovery.** Ванильный MCP-клиент (`@modelcontextprotocol/inspector`) подключается к `mcp.mnemonik.xyz` без Bearer-токена и получает: ≥7 entries в `prompts/list` с непустыми descriptions, ≥7 entries в `resources/list` с читаемыми bodies, и `tools/list` где каждый из 5 tool'ов несёт enriched description, ссылающуюся на соответствующий skill manifest.
- [ ] **AC2 — Manifest integrity.** Каждый prompt/resource происходит из одного markdown-файла. CI валит build если manifest упоминается без существующего исходного файла.
- [ ] **AC3 — Local sign offline.** С установленным CLI и bootstrapped identity, `sign_memory { mode: "local", visibility: "private", content: ... }` через mcp-stdio даёт валидный COSE_Sign1, INSERT в `~/.mnemonic/attestations.db` с `write_mode=local, visibility=private`, и **ноль сетевых вызовов** во время операции (netns assertion). Завершается < 5s при уже-загруженной модели.
- [ ] **AC4 — Local recall finds local-written.** Cosine search в local SQLite возвращает только что записанный attestation как top-1.
- [ ] **AC5 — Soft-fall semantics (public).** С `mode=local + visibility=public` и недоступным локальным embedder'ом + network available: транспарентно роутится на participate, OAuth-loopback если нужно, stderr-warning о public chain anchor'е, sign успешен с `write_mode=participate`.
- [ ] **AC6 — Privacy preservation.** С `mode=local + visibility=private` и недоступным embedder'ом: fail loud (`-32098 EmbedderInvalid`). Никаких publishments. Privacy contract не negotiable.
- [ ] **AC7 — Install idempotent.** Re-run производит byte-identical configs.
- [ ] **AC8 — Install non-destructive.** На машине с 3 хостами + pre-populated unrelated MCP entries: install добавляет `mnemonik` entry в каждый, оставляет все остальные entries byte-identical (diff-verification).
- [ ] **AC9 — Install resilient.** На машине с 1 из 3 хостов: только этот хост модифицируется, отсутствующие пропускаются silently. Exit 0 даже если ни один не найден.
- [ ] **AC10 — OAuth-loopback flow.** Первый participate-mode write триггерит browser popup; token cached в OS keychain; subsequent writes до 1h TTL не запускают browser.
- [ ] **AC11 — Token in keychain.** К концу v1 работы `~/.mnemonic/token.json` удалён; token живёт в OS keychain (`xyz.mnemonik.token/default` на macOS, etc.). `mnemonic logout` удаляет keychain entry.
- [ ] **AC12 — Visibility-respecting anonymous recall.** Unauthenticated recall возвращает **только** `visibility="public"` строки. CI test seed'ит DB с private + public matching query, asserts что anonymous response содержит public и исключает private.
- [ ] **AC13 — Embedder parity (cross-language golden).** Для канонической fixture-строки CLI's transformers.js и server's fastembed производят byte-identical 384-dim vectors. CI golden test, mismatch валит release.
- [ ] **AC14 — Embedder version surface.** Сервер возвращает `embedder.model_id` + `embedder.model_version` в `initialize` response; CLI логирует warning при mismatch.
- [ ] **AC15 — Tool descriptions inline manifests.** `tools/list` descriptions содержат `Purpose` + `Trigger` sections манифеста, вшитые на build-time. Drift между tools/list description и prompt/resource body **физически невозможен** (один markdown-источник).
- [ ] **AC16 — Error catalogue.** Все определённые ошибки (`-32095..-32099`) пропагируются через mcp-stdio как JSON-RPC error objects с documented data fields. Skills документируют их так, чтобы агент закладывал retry/surface logic на generation time, не на runtime surprise.

**Post-launch metrics (не gates):**

- В течение 14 дней после релиза ≥N уникальных pubkey'ев исполняют `mnemonic install` и ≥M из них делают как минимум один local-mode write. Anonymous discovery hits на hosted сервере трекаются отдельно. N/M фиксируются после первой недели данных.

## Ограничения

**Источник паттернов**: `github.com/aitankfish/pnl` (`@pnlmarket/mcp-server` v0.5.1, MIT). Заимствуем **паттерны** (npm-bin-is-installer-and-server, hardcoded host candidates, non-destructive JSON merge, skill-file copying). Код **не копируем**. Centralized admin model из PNL — **отвергаем явно**.

**Архитектурный контракт (locked-in):**

- Local-mode операции полностью локальны. Никаких сетевых вызовов. Памяти не покидают машину пользователя в local mode. (Путь B выбран над Путём A "hosted accepts anonymous COSE" — privacy + чтобы не нагружать shared сервер.)
- Embedder model parity: CLI использует `Xenova/all-MiniLM-L6-v2` через transformers.js, версия pinned. **Должно** совпадать с moделью server'а (fastembed MiniLM-L6, 384-dim) — иначе cross-mode recall не consistent. Проверяется CI golden test'ом.
- Identity для local-mode signing — переиспользуем invisible-bootstrap Ed25519 keypair (`~/.mnemonic/identity.json` + OS keychain, шипнуто сегодня в PR #154/#157).
- Discovery methods (`prompts/*`, `resources/*`, `tools/list`) **обязаны** succeed без Bearer'а. Write-tier tool calls сохраняют OAuth **только** в participate mode; local mode не требует OAuth вообще.
- Anonymous recall фильтруется `visibility='public'` — writer **явно** соглашается на публикацию через флаг. Это сильнее, чем "writer responsibility" из ранних версий идеи.
- **NO admin override / god-key anywhere.** Отличительная фича vs PNL — **отсутствие** `emergency_drain_vault`-эквивалента для attestation state. Hard rule.

**Scope OUT для v1 (явные non-goals):**

- NO autosign / encrypted-wallet / bound-challenge (T3 Part B из source brief). Write tools сохраняют OAuth + browser-mediated signing в participate mode.
- NO Cline, Codex, Windsurf поддержка в `install` (PNL's installer handle'ит только 3 хоста несмотря на overstatement в README; мы не повторяем).
- NO Linux/Windows host-config paths в install candidates() — macOS-only для v1.
- NO host-specific behavioural verification в release gate. Тест "agent attests by reflex" — post-launch.
- NO cross-mode recall в v1 — local recall и participate recall изолированы. Документируется в манифестах.

**Совместимость:**

- `mcp.mnemonik.xyz` сохраняет OAuth 2.1 + PKCE, modes-user-choice (#156, шипнут сегодня), browser-mediated signing, DoS guard, delivery confirmation. **Добавляем** prompts/resources handlers + middleware adjustment для anonymous discovery.
- CLI расширяется; существующие 0.2.2 subcommand'ы (init, login, sign, recall, verify, whoami, prove, identity push-to-webapp) остаются backwards-compatible.

## Риски

- **R1 — Privacy regression через mode-mistake (severe).** Агент ставит `visibility=public` на приватный контент. **Митигация:** default `visibility=private`; server-side `-32095 PublicWriteRequiresConfirmation` gate; skill manifest требует user-confirmation для public; stderr-audit line на каждом participate-mode write.
- **R2 — Local identity / token compromise (medium).** **Митигация:** identity в OS keychain (через invisible-bootstrap); token теперь тоже в keychain (AC11); 1h token TTL; `mnemonic logout` для explicit cleanup. OS-level security contract документирован; physical-access compromise **не** защищаем.
- **R3 — Embedder model parity drift (medium).** **Митигация:** CI golden test (AC13); версия embedder'а сурфейсится в `initialize` (AC14); та же модель pinned через CLI + server для всех v1.x; документировано что cross-mode recall не consistent.
- **R4 — CLI install size (low).** transformers.js + ONNX runtime + model добавляет ~50-100MB total (модель качается on-demand). Acceptable для developer CLI; аналог `pip install` сопоставимого размера.
- **R5 — Native dependency install fail (low-medium).** better-sqlite3 нужна native compilation на первой установке. **Митигация:** ship platform-specific prebuilt binaries через npm optionalDependencies; документировать fallback на pure-JS sqlite если нужно.
- **R6 — Host config drift (medium, future).** Хосты могут поменять config paths или схему. **Митигация:** хардкодный candidates() с file-existence guard'ом; install idempotent чтобы re-run ловил изменения; новые пути добавляются в CLI patch releases.
- **R7 — npm CLI version mismatch с hosted server (low-medium).** Старый CLI может не понять новых server responses. **Митигация:** server-side embedder version surface (AC14); CLI announce'ит свою версию в `initialize`; backwards-compat от server для одного minor cycle.
- **R8 — Skill manifest content quality drives adoption (high).** Плохо написанные манифесты вызывают over-attest или under-attest. **Митигация:** trigger boundary явные в `mnemonik-attest`; positive AND negative examples; dogfooding с internal review перед release.

## Технические решения

- **Решили: единый markdown — три MCP-проекции.** Manifests живут как `assets/skills/*.md` (или эквивалент). На build-time секции `Purpose` + `Trigger` extract'ятся в Rust string constants для `tools/list` descriptions. `prompts/list` отдаёт первый абзац как description, full body в `messages[0].content`. `resources/list` отдаёт entire markdown as `text/markdown`. Drift между поверхностями физически невозможен.
- **Решили: local-mode = local-everything.** Никаких сетевых вызовов в local mode. WASM core + transformers.js embedder + better-sqlite3. Памяти не покидают машину. (Отвергли альтернативу "hosted accepts anonymous COSE" — privacy + не нагружаем shared сервер.)
- **Решили: visibility flag separate от write_mode.** `write_mode` (local/participate) — куда писать. `visibility` (private/public) — можно ли публиковать. Soft-fall разрешён только когда `visibility=public`. Default `visibility=private` — privacy-by-default.
- **Решили: на model download fail с `visibility=private` — fail loud, без soft-fall.** Privacy contract не negotiable. С `visibility=public` — soft-fall разрешён.
- **Решили: token storage в OS keychain.** Сегодня `~/.mnemonic/token.json` — plaintext, тогда как identity уже в keychain. Закрываем аномалию. macOS Keychain / Linux Secret Service / Windows DPAPI через `keyring` crate (Rust) / `@napi-rs/keyring` (Node).
- **Решили: 3 хоста macOS-only для v1.** Повторяем реальное покрытие PNL, не overstatement из их README. Linux/Windows + Cline/Codex/Windsurf — v1.1+.
- **Решили: host restart accepted, не делаем hot-reload.** macOS notifications / SIGUSR1 / AppleScript — brittle, требуют permissions, цена > выгода. Install output говорит "please restart your agent if it's running."
- **Не делаем: pre-download embedder при install.** Пользователь может использовать только локально или только participate. Не тратим bandwidth до первой нужды. Opt-in `mnemonic install --eager-embedder` — потенциальный v1.1 follow-up.

## Тестирование

**Unit-тесты:** делаются всегда, не обсуждаются. В частности:

- Manifest files валидно парсятся через include_dir!/rust-embed
- prompts/list и resources/list handlers возвращают MCP-spec-conformant JSON
- Install JSON read/merge/write preserves unrelated keys (table-driven с synthetic configs)
- Install idempotent (re-run produces byte-identical output)
- mcp-stdio router: local mode dispatches local sign/recall, participate mode dispatches hosted proxy
- Embedder parity: canonical fixture string → byte-identical embeddings в CLI и server (cross-language golden)

**Интеграционные тесты:** делаем — критичные для release gate (AC1, AC3, AC10, AC12, AC15).

- Spin up mcp-server локально, hit от reqwest клиента без Bearer'а, verify discovery surface (AC1)
- Mode-routing: local sign_memory не делает сетевых вызовов вне proxy boundary (netns assertion — AC3)
- OAuth-loopback фирится только когда participate mode + no cached token (AC10)
- Anonymous recall filter (AC12)

**E2E тесты:** **не делаем** для release gate — host-specific behavioural verification (Claude Code / Cursor / Claude Desktop scripted sessions) отложено на post-launch. Causa: дорого по hardware + brittle к host updates. Smoke test (manual MCP Inspector smoke + fresh-machine install + airplane-mode local write) выполняется перед release вручную.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. Anonymous discovery возвращает 7 prompts | `curl https://mcp.mnemonik.xyz/mcp` с JSON-RPC `prompts/list` без Authorization | HTTP 200; JSON-RPC result содержит ≥7 prompts с непустыми descriptions |
| 2. Anonymous discovery возвращает 7 resources | то же с `resources/list` | ≥7 resources, каждый имеет валидный URI и readable body |
| 3. tools/list содержит enriched descriptions | то же с `tools/list` | Каждый из 5 tool'ов имеет description ≥500 байт включая `Purpose:` и `Trigger:` секции |
| 4. Anonymous recall returns only public | seed DB с private + public matching test query, query через mcp.mnemonik.xyz `tools/call recall` без Bearer | Только public row в response |
| 5. Install idempotent | bash: pre-populate ~/.claude.json с unrelated entry, run `mnemonic install`, dump file, run снова, diff | Byte-identical между двумя dumps; unrelated entry неизменён |
| 6. Local sign offline | bash в netns без интернета: `mnemonic sign --local --visibility private "test content"` | Exit 0; row в ~/.mnemonic/attestations.db с write_mode=local, visibility=private; никаких outbound TCP |
| 7. Local recall finds local-written | bash: запись через шаг 6, потом `mnemonic recall --local "test content"` | Returns top-1 = только что записанный attestation |
| 8. Privacy-private fails loud on model fail | bash: rm -rf ~/.mnemonic/models/; `mnemonic sign --local --visibility private "x"` | Exit 1 (или другой error); error -32098 EmbedderInvalid; никаких outbound calls |
| 9. Embedder parity golden | CI: run server fastembed + CLI transformers.js на канонической fixture-строке, compare bytes | Match exact |
| 10. Token в keychain после login | bash: `mnemonic login`, then `security find-generic-password -s xyz.mnemonik.token -a default` (macOS) | Returns entry; `~/.mnemonic/token.json` не существует |

### Пользователь проверяет

- **Fresh-machine install smoke (manual, перед release):** На чистой macOS машине: `npm install -g @mnemonik-xyz/cli && mnemonic install`. Открыть Claude Code, проверить что mnemonik tools видны в `/mcp` menu, попробовать local-mode attestation, проверить что работает в airplane mode (после первого скачивания модели). Зачем руками: behavioral observation (agent sees tools, agent invokes correctly) не покрывается CI.
- **MCP Inspector smoke (manual, перед release):** `npx @modelcontextprotocol/inspector https://mcp.mnemonik.xyz/mcp` без auth. Глазами проверить что 7 prompts / 7 resources visible. Screenshot в release-checklist. Зачем руками: визуальная проверка UX-полирования (descriptions не обрезаются, не выглядят странно).

## Follow-ups (v1.1+)

- Linux + Windows host-config paths в install candidates().
- Cline / Codex / Windsurf поддержка в install.
- Behavioural smoke matrix через 3 хоста (scripted-session "agent attests by reflex" тесты).
- Pre-download embedder опционально через `mnemonic install --eager-embedder` чтобы снизить first-write latency.
- Multi-version embedder support на server'е (один из путей закрытия R3, если model upgrade станет нужен).
