---
created: 2026-06-04
approved: 2026-06-04
status: approved
type: feature
size: L  # 3 coordinated pieces — server-side propagation + npm shim ship-Rust-binary + install subcommand + new visibility column on participate
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
2. Манифесты доставляются через **три MCP-поверхности** одновременно: `prompts/list + prompts/get`, `resources/list + resources/read`, и **enriched `tools/list` descriptions** для всех 5 существующих инструментов (Purpose + Trigger секции манифеста встраиваются на build-time).
3. **Discovery работает pre-auth.** `initialize`, `prompts/*`, `resources/*`, `tools/list` отвечают без Bearer-токена. Существующий OAuth-gate остаётся на write-tier tool calls в participate mode.
4. **Anonymous recall** работает без auth над публичной частью пула (см. `visibility` ниже).
5. Схема расширяется одной колонкой: `attestations.visibility TEXT NOT NULL DEFAULT 'private'` (values: `'private'` | `'public'`). Колонка применима **только** к строкам, записанным через participate mode. Anonymous recall возвращает **только** `visibility='public'` строки. Default `'private'` — privacy-by-default даже для participate.

**Кусок 2 — npm shim `@mnemonik-xyz/mcp` (новый package) + `mnemonik-mcp install` subcommand.**

6. Новый npm package `@mnemonik-xyz/mcp` — тонкий launcher по pattern'у esbuild / prisma / swc. На install (`npm install -g @mnemonik-xyz/mcp` или `npx -y @mnemonik-xyz/mcp install`) определяет платформу и скачивает соответствующий prebuilt **Rust binary** из GitHub Releases (тот же артефакт, что и для cargo-install). Скачанный binary верифицируется против checksum'а, эмитированного **той же tagged pipeline** что и собирал binary (release.yml должен emit'ить verified manifest — конкретный механизм tech-spec'а, может быть SHA256SUMS / sigstore / minisign). Binary кеширует в платформо-стандартный location под именем **`mnemonik-mcp`** (с `k`, согласовано с npm scope `@mnemonik-xyz`); artifact на GitHub Releases остаётся как `mnemonic-mcp` для backwards-compat с `cargo install`. Shim делает rename/symlink в кэше.
7. **`mnemonik-mcp install`** — pattern PNL'я. Хардкоднутый список из 3 хост-конфигов: `~/.claude.json`, `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS), `~/.cursor/mcp.json`. Только-если-файл-существует, non-destructive JSON merge, идемпотентно.
8. В каждый существующий config пишется одна запись с ключом `mnemonik`, указывающая на **локально-распакованный путь к binary** (не `npx`, чтобы не пинговать registry при каждом старте хоста) и subcommand `mcp-stdio`. Все остальные ключи и MCP-серверы остаются byte-identical. Re-run заменяет в-месте, дубликатов нет.
9. **`mnemonik-mcp install --check`** — dry-run. Показывает: какие хост-конфиги обнаружены, в какие нужно записать `mnemonik`-entry, какие уже содержат `mnemonik`-entry (replacement-target). Никаких изменений на диске.
10. Output после `install` (apply) говорит пользователю **перезапустить уже запущенные агенты**, если они работали в момент установки. Если хост не запущен — открыть его как обычно.
11. **`mnemonik-mcp doctor`** — диагностическая команда. Проверяет: presence `mnemonik` записи в каждом хост-конфиге, ping `mcp.mnemonik.xyz/health`, проверка binary integrity (hash), доступность local SQLite read/write, keychain accessibility for identity + token. Pass/fail per check + repair hints.

**Кусок 3 — `mnemonik-mcp mcp-stdio` (новая subcommand'а binary'я).**

12. Existing Rust `mnemonic-mcp` binary получает новую subcommand'у `mcp-stdio`. JSON-RPC поверх stdin/stdout, спавнится хостом. Подразумевается использование существующего MCP server core'а (axum/JSON-RPC dispatch, tools registry).
13. **Dual-route по `mode`-аргументу tool call'а** (см. modes-user-choice #156):
    - **Local mode (default)**: полное локальное исполнение **через тот же Rust код**, который сейчас работает на `mcp.mnemonik.xyz`. Existing core embedding + compression + canonical CBOR + COSE_Sign1 pipeline — никакого переписывания. INSERT в локальный SQLite. **Никаких сетевых вызовов** в local mode. Подпись локальным Ed25519 identity (из invisible-bootstrap, PR #154/#157).
    - **Participate mode**: проксирует JSON-RPC в `mcp.mnemonik.xyz/mcp` через HTTPS. На первый auth-required вызов запускает OAuth-loopback (browser popup).
    - **Discovery (`prompts/*`, `resources/*`, `tools/list`)**: отвечает **из binary напрямую**, не проксирует. Manifests baked в binary через `include_dir!` на build-time (та же source-of-truth, что и server использует). Это позволяет агенту видеть skills даже offline / при недоступности hosted server'а. Server-side manifests и binary-embedded snapshot обновляются вместе через release pipeline; mismatch (старый binary, новый server) сёрфейсится через `embedder.model_version`-style сравнение в `initialize` response.
14. **Token storage переезжает в OS keychain.** Сегодня `~/.mnemonic/token.json` — plaintext. После v1 — token живёт в OS keychain (точные координаты — tech-spec, использует существующую keychain-инфраструктуру). Это закрывает аномалию: identity у нас в keychain через invisible-bootstrap, токен сейчас — нет. `mnemonik-mcp logout` удаляет keychain entry. **Migration**: при первом login через mcp-stdio, если существующий `~/.mnemonic/token.json` найден — token читается, записывается в keychain, файл удаляется. Один shot, no-op на subsequent login'ах. Existing `@mnemonik-xyz/cli` (Node) v0.2.x продолжает работать с keychain entry — его читает через тот же `@napi-rs/keyring` (уже используется для identity). Не требует одновременного релиза CLI и shim'а.
15. **Soft-fall = explicit opt-in (не silent).** На сбой local embedder'а / fs / sqlite:
    - **Default behaviour**: возвращается типизированная JSON-RPC ошибка (точные коды — tech-spec). Никакого silent escalation. Агент видит ошибку и может либо retry, либо surface user'у, либо отдельным вызовом запросить participate-mode (что вызовет OAuth-loopback).
    - **Explicit opt-in**: tool args принимают `allow_fallback_to_participate: true`. Default `false`. Если `true` и local fails — mcp-stdio автоматически роутит на participate; в response добавляется поле `escalated: { from: "local", to: "participate", reason: ... }` — агент **видит** escalation в самом response, не только в stderr.
    - **Identity-bootstrap failure** (no keychain, no file fallback — рассмотрено в invisible-identity PR #154/#157, но edge cases возможны): тоже loud error. Local mode не способен подписать без identity.
16. **Concurrent local writes**: SQLite в WAL-режиме с busy timeout. Несколько MCP-хостов (Claude Code + Cursor одновременно) → каждый shim-процесс retry'ит на лоак-контеншне до короткого budget'а (точные значения — tech-spec). На превышение budget'а — типизированная ошибка `LocalStorageBusy`, агент retry'ит.

## Зачем

**Сегодня протокол установлен, но не используется.**

Даже после успешной ручной настройки (отредактировал `~/.claude.json`, прошёл OAuth) агент не дотягивается до инструментов в правильные моменты: пять tool'ов отдаются с минимальными descriptions, никаких behavioural-инструкций. Агент не знает когда attest'ить (после каких решений, не каких нажатий клавиш), что не attest'ить (transient state, scratch work, PII), как собрать payload context из диалога, какие guardrails применять. В результате — протокол установлен, но не работает по cadence'у, который превращает его в habit.

**Сегодня даже установка непропорционально болезненна.**

Чтобы попробовать Mnemonic нужно: найти docs, вручную добавить config-entry в свой агент, перезапустить агента, пройти OAuth-флоу, **и только потом** агент видит инструменты — без behavioural-инструкций. Пользователь, который сегодня хочет local-mode attestations (не платный, не публичный), всё равно обязан пройти полный OAuth-цикл для **любого** sign'а.

**Цель этой фичи:**

- **Установка в две команды**: `npm install -g @mnemonik-xyz/mcp && mnemonik-mcp install`. После рестарта (или первого старта) агент видит mnemonik. Никаких ручных правок конфигов.
- **Local-mode usage без OAuth и без сети**: подписать память локально, никаких prompts, никаких сетевых хапов. Никакого WASM-cold-start: используется тот же compiled Rust core, что и hosted server, через нативный binary.
- **Anonymous discovery до OAuth**: подключённый агент видит skills + recall публичной части пула без идентичности вообще. Это безопасно потому что (a) только `visibility='public'` строки выходят в anonymous recall (privacy-by-default — приватные writes invisible'ы), (b) никакого write capability для анонимного клиента. Это превращает MCP-endpoint в inbound distribution channel.
- **OAuth срабатывает только когда явно нужен** — на первый participate-mode write.

## Целевая аудитория

Любой MCP-capable агент или его оператор. Две когорты:

1. **Anonymous / curious**: подключается к `mcp.mnemonik.xyz` без auth, изучает что протокол даёт через prompts и resources, может попробовать anonymous recall публичной части пула. Решает OAuth-init для записи или просто узнаёт.
2. **Authenticated operator**: установил CLI через `mnemonik-mcp install`, ожидает что local-mode писалки работают без вопросов, participate-mode требует OAuth и платёж.

Поддерживаемые хосты в v1: **Claude Code, Claude Desktop (macOS), Cursor**. Linux / Windows пути, Cline / Codex / Windsurf — v1.1+.

## Как должно работать

**Поток 1 — Newcomer install.**

1. Пользователь делает `npm install -g @mnemonik-xyz/mcp` (или `npx -y @mnemonik-xyz/mcp install`). npm shim определяет платформу, качает prebuilt `mnemonic-mcp` Rust binary из GitHub Releases, верифицирует hash, кеширует.
2. Пользователь делает `mnemonik-mcp install`. CLI находит существующие хост-конфиги, пишет в каждый одну `mcpServers.mnemonik` запись с **прямым путём к binary** (не `npx`), не трогает другие ключи, не создаёт конфиги для незаустановленных хостов. Output: «Mnemonik wired into Claude Code, Cursor. If any of them is already running, please restart it.»
3. Пользователь открывает Claude Code. Хост спавнит `/path/to/mnemonik-mcp mcp-stdio` как subprocess. Subprocess advertise'ит 5 tools + 7 prompts + 7 resources хосту (manifests baked в binary, не требуют network).
4. Хост exposуит `/mnemonik-*` slash-commands и инструменты агенту с полными descriptions.
5. Агент видит инструменты + skill-manifests и реагирует ими в правильные моменты — никаких дополнительных действий от пользователя.

**Поток 2 — First local-mode write (offline-capable).**

1. Агент решает аттестовать (скажем, после committed code change — что описано в `mnemonik-attest` манифесте).
2. Агент вызывает `sign_memory { mode: "local", content: ... }` через mcp-stdio. (visibility не указывается — local подразумевает private.)
3. mcp-stdio (= Rust binary с `mcp-stdio` subcommand): invisibly обеспечивает identity (через invisible-bootstrap из PR #154/#157, no-op если уже есть). Дальше тот же исполнительный path, что и hosted server в local mode: embed через fastembed, TurboQuant compress, canonical CBOR, COSE_Sign1 sign, INSERT в `~/.mnemonic/attestations.db`. **Никаких сетевых вызовов.**
4. Агент получает подтверждение `{attestation_id, content_hash, write_mode: "local"}`.
5. Пользователь не видит ничего — silent.

**Поток 3 — First participate-mode write (public chain-anchored).**

1. Агент решает зааттестовать что-то значимое и предлагает пользователю опубликовать. Skill manifest `mnemonik-attest` требует confirmation от пользователя для `visibility: "public"`.
2. Пользователь подтверждает. Агент вызывает `sign_memory { mode: "participate", visibility: "public", content: ... }`.
3. mcp-stdio проксирует на `mcp.mnemonik.xyz`. Сервер видит no Bearer → возвращает auth challenge. mcp-stdio запускает OAuth-loopback: открывает браузер, пользователь логинится через webapp.
4. Token cached в OS keychain. Запись anchor'ится на Solana + Arweave. Stderr line: `[mnemonik] participate-mode write: <pubkey> <content_hash> visibility=public`.
5. На follow-up participate-mode write'ы token уже в keychain → OAuth не запускается, до 1h TTL.

**Поток 4 — Anonymous discovery (без CLI).**

1. Разработчик добавляет `https://mcp.mnemonik.xyz/mcp` напрямую в `~/.cursor/mcp.json` (без установки CLI).
2. Cursor подключается без Bearer. Сервер пропускает discovery surface; `prompts/list`, `resources/list`, `tools/list` все возвращаются.
3. Разработчик или его агент пробуют `recall` запрос — сервер фильтрует по `visibility='public'` и возвращает публичную часть пула.
4. Если решает participate, следует skill-manifest `mnemonik-init`, который проводит через `npm install -g @mnemonik-xyz/mcp && mnemonik-mcp install`.

## Критерии приёмки

**Release gate (должно держаться для shippa — protocol-level, без host-specific behavioural verification):**

- [ ] **AC1 — Anonymous discovery.** Ванильный MCP-клиент (`@modelcontextprotocol/inspector`) подключается к `mcp.mnemonik.xyz` без Bearer-токена и получает: ≥7 entries в `prompts/list` с непустыми descriptions, ≥7 entries в `resources/list` с читаемыми bodies, и `tools/list` где каждый из 5 tool'ов несёт enriched description, ссылающуюся на соответствующий skill manifest.
- [ ] **AC2 — Manifest integrity.** Каждый prompt/resource происходит из одного markdown-файла. CI валит build если manifest упоминается без существующего исходного файла. `Purpose` + `Trigger` секции автоматически инжектируются в `tools/list` descriptions на build-time, drift между поверхностями физически невозможен.
- [ ] **AC3 — Local sign offline.** С распакованным binary и bootstrapped identity, `sign_memory { mode: "local", content: ... }` через mcp-stdio даёт валидный COSE_Sign1, INSERT в `~/.mnemonic/attestations.db` с `write_mode=local`, и **ноль outbound сетевых вызовов** во время операции. Тестируется в network-namespace без какой-либо internet connectivity (как airplane mode). Завершается < 1s на нормальной машине.
- [ ] **AC4 — Local recall finds local-written.** Cosine search в local SQLite возвращает только что записанный attestation как top-1.
- [ ] **AC5 — Default behaviour: no silent escalation.** С `mode=local` + недоступным local embedder'ом + **БЕЗ** `allow_fallback_to_participate: true` arg: возвращается типизированная JSON-RPC ошибка (embedder-invalid family) независимо от состояния сети. Никаких publishments, никакого OAuth popup'а, никаких outbound сетевых вызовов. Privacy contract non-negotiable.
- [ ] **AC6 — Explicit opt-in fallback semantics.** С `mode=local` + `allow_fallback_to_participate: true` + недоступным local embedder + network available: транспарентно роутится на participate, OAuth-loopback если нужен, **в JSON-RPC response** содержится `escalated: { from: "local", to: "participate", reason: ... }`. Stderr line warns о chain anchor'е. С `allow_fallback_to_participate: true` + no network: возвращается hosted-unavailable error (escalation провалилась), а не embedder-invalid — агент видит точную причину failure.
- [ ] **AC7 — Install idempotent.** Re-run производит byte-identical configs.
- [ ] **AC8 — Install non-destructive.** На машине с 3 хостами + pre-populated unrelated MCP entries: install добавляет `mnemonik` entry в каждый, оставляет все остальные entries byte-identical (diff-verification).
- [ ] **AC9 — Install resilient.** На машине с 1 из 3 хостов: только этот хост модифицируется, отсутствующие пропускаются silently. Exit 0 даже если ни один не найден.
- [ ] **AC10 — Install --check (dry-run).** `mnemonik-mcp install --check` печатает план (какие хост-конфиги обнаружены, какие будут модифицированы, какие уже содержат mnemonik-entry как replacement target). Exit 0. На диск ничего не пишется (verified via mtime comparison before/after). По умолчанию `mnemonik-mcp install` без флагов **апплаит** изменения (write-by-default, dry-run — opt-in).
- [ ] **AC11 — OAuth-loopback flow.** Первый participate-mode write триггерит browser popup; token cached в OS keychain (точные координаты — tech-spec); subsequent writes до 1h TTL не запускают browser.
- [ ] **AC12 — Token in keychain.** После любой participate-mode операции `~/.mnemonic/token.json` отсутствует; token живёт **только** в OS keychain. `mnemonik-mcp logout` удаляет keychain entry. Тест: asserting file absence + `mnemonik-mcp doctor` report token-in-keychain = pass.
- [ ] **AC13 — Visibility-respecting anonymous recall.** Unauthenticated recall возвращает **только** `visibility="public"` строки. CI test seed'ит DB одной private + одной public записью matching query string, asserts что anonymous response содержит public и исключает private.
- [ ] **AC14 — Visibility only on participate writes.** `sign_memory { mode: "local", visibility: "public" }` отвергается типизированной invalid-params ошибкой. Local writes неявно приватны (нет колонки на disk, нет concept'а sharing).
- [ ] **AC15 — Embedder configuration surface.** Сервер возвращает `embedder.model_id` + `embedder.model_version` в `initialize` response. При connect mcp-stdio сравнивает с собственными values (pinned compile-time в Rust binary); на mismatch — stderr warning `[mnemonik-mcp] embedder version mismatch: local=<x> server=<y>, cross-mode recall not guaranteed consistent`. Testable: integration test инжектит mismatch и asserts warning line.
- [ ] **AC16 — Error catalogue propagation.** Все определённые типизированные ошибки пропагируются через mcp-stdio как JSON-RPC error objects с documented data fields. Конкретно testable: для каждой ошибки в каталоге (точный list — tech-spec) есть integration test, который триггерит условие и asserts что возвращены code + data shape соответствуют контракту.
- [ ] **AC17 — Doctor command.** `mnemonik-mcp doctor` проверяет: presence `mnemonik` записи в каждом хост-конфиге, ping `mcp.mnemonik.xyz/health`, binary integrity (hash matches expected), local SQLite read/write, identity accessibility (через invisible-bootstrap), keychain accessibility for token (если participate было). Output: structured pass/fail per check + repair hints. Exit 0 если все pass, exit 1 если есть fail. Testable: на сломанном keychain выдаёт fail с repair-hint.

**Post-launch metrics (не gates, informational baseline):**

- В течение 14 дней после релиза собираются три метрики: (1) уникальные pubkey'и, исполнившие `mnemonik-mcp install` хотя бы раз; (2) из них процент сделавших хотя бы один local-mode write; (3) anonymous discovery hits на hosted сервере. Конкретные пороги "успешного" релиза не фиксируем заранее — после первой недели данных команда фиксирует baseline и решает что считать roadmap pivot signal'ом.

## Ограничения

**Источник паттернов**: `github.com/aitankfish/pnl` (`@pnlmarket/mcp-server` v0.5.1, MIT). Заимствуем **паттерны** (npm-bin-is-installer-and-server, hardcoded host candidates, non-destructive JSON merge, skill-file copying). Код **не копируем**. Centralized admin model из PNL — **отвергаем явно**.

**Архитектурный контракт (locked-in):**

- Local-mode операции полностью локальны. Никаких сетевых вызовов. Памяти не покидают машину пользователя в local mode. (Путь B "переписать в Node" отвергнут — Path C "ship existing Rust binary через npm shim" выбран. Преимущество: переиспользуем уже-протестированный Rust core, тот же fastembed embedder, тот же canonical-CBOR/COSE, тот же SQLite schema. Нет cross-runtime parity вопросов. Нет WASM cold-start. Нет реимплементации.)
- Identity для local-mode signing — переиспользуем invisible-bootstrap Ed25519 keypair (`~/.mnemonic/identity.json` + OS keychain, шипнуто сегодня в PR #154/#157).
- Discovery methods (`prompts/*`, `resources/*`, `tools/list`) **обязаны** succeed без Bearer'а. Write-tier tool calls сохраняют OAuth **только** в participate mode; local mode не требует OAuth вообще.
- Soft-fall = explicit opt-in. По умолчанию `mode=local` отказывается с loud error при сбое. `allow_fallback_to_participate: true` — единственный путь к escalation, и agent видит escalation в response.
- Visibility flag применим **только** к participate-mode writes. Local writes неявно приватны. Anonymous recall фильтруется `visibility='public'`. Default `visibility='private'` даже для participate — privacy-by-default.
- **NO admin override / god-key anywhere.** Отличительная фича vs PNL — **отсутствие** `emergency_drain_vault`-эквивалента для attestation state. Hard rule.

**Scope OUT для v1 (явные non-goals):**

- NO autosign / encrypted-wallet / bound-challenge (T3 Part B из source brief). Write tools сохраняют OAuth + browser-mediated signing в participate mode.
- NO Cline, Codex, Windsurf поддержка в `install` (PNL's installer handle'ит только 3 хоста, despite README overstatement; мы не повторяем).
- NO Linux/Windows host-config paths в install candidates() — macOS-only для v1.
- NO host-specific behavioural verification в release gate. Тест "agent attests by reflex" — post-launch.
- NO cross-mode recall в v1 — local recall и participate recall изолированы. Документируется в манифестах.
- NO Node-side реимплементация core логики. Используем существующий Rust binary через npm shim.

**Совместимость:**

- `mcp.mnemonik.xyz` сохраняет OAuth 2.1 + PKCE, modes-user-choice (#156, шипнут сегодня), browser-mediated signing, DoS guard, delivery confirmation. **Добавляем** prompts/resources handlers + middleware adjustment для anonymous discovery + visibility column.
- CLI (Node `@mnemonik-xyz/cli`) остаётся как есть для interactive use. Новый npm shim `@mnemonik-xyz/mcp` отдельный package, на нём держится install/mcp-stdio механизм.

## Риски

- **R1 — Privacy regression через mode-mistake (severe).** Агент ставит `visibility=public` на приватный контент. **Митигация:** default `visibility=private` даже в participate; visibility отсутствует в local mode полностью (AC14); server-side public-write confirmation gate (typed error); skill manifest требует user-confirmation для public writes; stderr-audit line на каждом participate-mode write.
- **R2 — Soft-fall escalation surprise (medium).** Agent или operator думает что local-mode не делает чего-то — но `allow_fallback_to_participate: true` всё равно эскалирует. **Митигация:** opt-in default `false`; escalation возвращается в JSON-RPC response (не только stderr) — agent видит изменение и может решить как surface'ить юзеру.
- **R3 — Local identity / token compromise (medium).** **Митигация:** identity в OS keychain (через invisible-bootstrap); token теперь тоже в keychain (AC12); 1h token TTL; `mnemonik-mcp logout` для explicit cleanup. OS-level security contract документирован; physical-access compromise **не** защищаем.
- **R4 — Binary distribution complexity (low-medium).** Прибавляется новый npm package `@mnemonik-xyz/mcp` который скачивает Rust binary на install. release.yml сейчас имеет проблемы с Linux builds (libdbus, отдельная задача) — необходимо чтобы macOS builds работали стабильно для v1 (already в порядке). Linux/Windows v1.1. **Митигация:** release.yml должен emit'ить verified checksum manifest (SHA256SUMS или sigstore signature) от той же tagged pipeline, что и собирает binary. shim верифицирует hash на download. На failure — clear error message; fallback не предусматриваем (no Rust binary = no install).
- **R5 — Host config drift (medium, future).** Хосты могут поменять config paths или схему. **Митигация:** хардкодный candidates() с file-existence guard'ом; install idempotent чтобы re-run ловил изменения; новые пути добавляются в patch releases.
- **R6 — npm shim version mismatch с binary в кэше (low).** Пользователь обновляет `@mnemonik-xyz/mcp` package, но в кэше старый binary. **Митигация:** на каждый запуск shim проверяет binary version против expected (стейтлесс check). Mismatch → re-download.
- **R7 — Skill manifest content quality drives adoption (high).** Плохо написанные манифесты вызывают over-attest или under-attest. **Митигация:** trigger boundary явные в `mnemonik-attest`; positive AND negative examples; dogfooding с internal review перед release. Error-handling guidance (retry/surface patterns для каждой типизированной ошибки из каталога) встроена в manifests — чтобы агент знал что делать при failure на generation time.
- **R8 — Concurrent local writes contention (low).** Multiple host subprocesses (Claude Code + Cursor одновременно) пишут в один SQLite. **Митигация:** WAL mode + bounded busy-timeout retry. На превышение budget'а — типизированная "busy" ошибка, агент retry'ит. Точные значения — tech-spec.
- **R9 — Identity-bootstrap edge case в local mode (low).** Если invisible-bootstrap (PR #154/#157) failed (corrupted keychain без file fallback), local sign не может подписать. **Митигация:** типизированная "identity-bootstrap-failed" ошибка с repair-hint; `mnemonik-mcp doctor` диагностирует и предлагает шаги.

## Технические решения

- **Решили: единый markdown — три MCP-проекции.** Manifests = единый источник правды per skill. На build-time секции "Purpose" + "Trigger" extract'ятся в строковые константы для `tools/list` descriptions. Prompts и resources surfaces отдают весь markdown без сокращений. Drift физически невозможен.
- **Решили: Path C — ship existing Rust binary через npm shim.** Не переписываем local-mode pipeline в Node. Используем тот же compiled Rust core, что и hosted server. Pattern as esbuild/prisma/swc. Преимущество vs Path B (Node реимплементация): нет cross-runtime embedder parity вопросов; нет WASM cold-start; нет реимплементации криптографических примитивов; меньше maintenance surface.
- **Решили: visibility flag только в participate mode.** Local writes неявно приватны (никогда не покидают машину). Visibility имеет смысл только для writes которые видны кому-либо кроме owner'а. Default private даже для participate.
- **Решили: soft-fall = explicit opt-in.** Local mode не эскалирует молча. `allow_fallback_to_participate: true` — явный opt-in, escalation видна в JSON-RPC response. Без opt-in — loud error.
- **Решили: token storage в OS keychain.** Сегодня `~/.mnemonic/token.json` — plaintext, тогда как identity уже в keychain. Закрываем аномалию.
- **Решили: 3 хоста macOS-only для v1.** Повторяем реальное покрытие PNL. Linux/Windows + Cline/Codex/Windsurf — v1.1+.
- **Решили: host restart accepted, не делаем hot-reload.** Цена > выгода. Install output говорит "please restart your agent if it's running."
- **Решили: install пишет прямой путь к binary, не `npx -y`.** PNL использует `npx -y` потому что у них нет prebuilt binary. У нас есть. `npx -y` пингует registry на каждом старте хоста — нарушает offline contract (AC3).
- **Решили: install по умолчанию апплаит изменения, --check — opt-in dry-run.** Default — самое частое user action; dry-run — диагностика.
- **Решили: token migration one-shot.** Первый participate flow через mcp-stdio мигрирует существующий plaintext file в keychain. Не требует одновременного coordinated release'а с existing `@mnemonik-xyz/cli`.
- **Не делаем: pre-download embedder при install.** Эта проблема не существует с Path C — embedder уже встроен в binary. Bandwidth на install = только сам binary.

## Тестирование

**Unit-тесты:** делаются всегда, не обсуждаются. В частности:

- Manifest files валидно парсятся через `include_dir!`/`rust-embed`
- prompts/list и resources/list handlers возвращают MCP-spec-conformant JSON
- Install JSON read/merge/write preserves unrelated keys (table-driven с synthetic configs)
- Install идемпотентен (re-run produces byte-identical output)
- mcp-stdio router: local mode dispatches local execution через core, participate mode dispatches hosted proxy, discovery всегда proxy
- Soft-fall semantics: `allow_fallback_to_participate=false` → loud typed error; `true` + network available → escalation + response field; `true` + no network → typed hosted-unavailable error (not embedder-invalid)
- Visibility validation: `mode=local + visibility set` → typed invalid-params error

**Интеграционные тесты:** делаем — критичные для release gate (AC1, AC3, AC11, AC13, AC17).

- Spin up mcp-server локально, hit от reqwest клиента без Bearer'а, verify discovery surface (AC1)
- Mode-routing: local sign_memory не делает сетевых вызовов вне proxy boundary (netns assertion — AC3)
- OAuth-loopback фирится только когда participate mode + no cached token (AC11)
- Anonymous recall filter (AC13)
- Install идемпотентность + non-destructive merge (AC7, AC8)

**E2E тесты:** **не делаем** для release gate — host-specific behavioural verification (Claude Code / Cursor / Claude Desktop scripted sessions) отложено на post-launch. Smoke test (manual MCP Inspector + fresh-machine install + airplane-mode local write) выполняется перед release вручную.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. Anonymous discovery возвращает 7 prompts | `curl https://mcp.mnemonik.xyz/mcp` с JSON-RPC `prompts/list` без Authorization | HTTP 200; JSON-RPC result содержит ≥7 prompts с непустыми descriptions |
| 2. Anonymous discovery возвращает 7 resources | то же с `resources/list` | ≥7 resources, каждый имеет валидный URI и readable body |
| 3. tools/list содержит enriched descriptions | то же с `tools/list` | Каждый из 5 tool'ов имеет description ≥500 байт включая `Purpose:` и `Trigger:` секции |
| 4. Anonymous recall returns only public | seed DB private + public matching test query, query через mcp.mnemonik.xyz `tools/call recall` без Bearer | Только public row в response |
| 5. Install idempotent | bash: pre-populate ~/.claude.json unrelated entry, run `mnemonik-mcp install`, dump file, run снова, diff | Byte-identical между dumps; unrelated entry неизменён |
| 6. Install --check is dry-run | bash: записать mtime до и после `mnemonik-mcp install --check` на каждый хост-конфиг | mtimes неизменны; stdout содержит план; exit 0 |
| 7. Local sign offline | bash в netns без интернета: `mnemonik-mcp mcp-stdio` спавнится, послать JSON-RPC `tools/call sign_memory { mode: "local", content: "test" }` | Result: успех; row в локальной DB с write_mode=local; никаких outbound TCP |
| 8. Default behaviour: no silent escalation | bash: повредить локальный embedder cache; послать sign_memory без `allow_fallback_to_participate` | Типизированная embedder-invalid ошибка; никаких outbound calls; никакого browser popup |
| 9. Explicit opt-in fallback works | то же что #8, но с `allow_fallback_to_participate: true` и интернетом | Success; response содержит `escalated: { from: "local", to: "participate" }`; stderr содержит warning line |
| 10. Visibility rejected on local | послать `sign_memory { mode: "local", visibility: "public" }` | Типизированная invalid-params ошибка |
| 11. Token в keychain после login | bash: через mcp-stdio инициировать participate write (что триггерит OAuth-loopback), потом `mnemonik-mcp doctor` | Doctor reports token-in-keychain = pass; файл `~/.mnemonic/token.json` отсутствует |
| 12. AC15: embedder config surface | JSON-RPC `initialize` request к mcp.mnemonik.xyz без Bearer | Result содержит `embedder.model_id` и `embedder.model_version` non-empty |
| 13. Doctor diagnoses keychain corruption | bash: revoke keychain access для test user; `mnemonik-mcp doctor` | Exit 1; failed check для "token-keychain-access"; repair-hint включает шаги |
| 14. Token migration from existing CLI | bash: имитировать наличие `~/.mnemonic/token.json` v0.2.x; запустить participate write через mcp-stdio | Файл удалён; token в keychain; existing `mnemonic` CLI читает его через `@napi-rs/keyring` без переустановки |

### Пользователь проверяет

- **Fresh-machine install smoke (manual, перед release):** На чистой macOS машине: `npm install -g @mnemonik-xyz/mcp && mnemonik-mcp install`. Открыть Claude Code, проверить что mnemonik tools видны в `/mcp` menu, попробовать local-mode attestation, проверить что работает в airplane mode (binary не требует internet, embedder встроен). Зачем руками: behavioral observation (agent видит tools, агент invok'ает корректно) не покрывается CI.
- **MCP Inspector smoke (manual, перед release):** `npx @modelcontextprotocol/inspector https://mcp.mnemonik.xyz/mcp` без auth. Глазами проверить что 7 prompts / 7 resources visible. Screenshot в release-checklist. Зачем руками: визуальная проверка UX-полирования (descriptions не обрезаются, не выглядят странно).

## Follow-ups (v1.1+)

- Linux + Windows host-config paths в install candidates() (заблокировано release.yml libdbus fix).
- Cline / Codex / Windsurf поддержка в install.
- Behavioural smoke matrix через 3 хоста (scripted-session "agent attests by reflex" тесты).
- Pre-anchored "soft-publish": запись изначально в local, потом отдельной командой promote до participate (без re-signing).
- Multi-version embedder support на server'е (если model upgrade станет нужен — закрывает R6 alt-path).
- **Move JWT token from `~/.mnemonic/token.json` to OS keychain.** Originally AC11+AC12 in v1 scope; deferred during tech-spec round 2 (2026-06-04) because tokens are short-lived (1h) and re-OAuth-on-expiry is cheap relative to the cost of coordinated Rust+Node keychain wrappers + keychain-unlock UX (intrusive on Linux Secret Service). Identity is already in keychain (PRs #154/#157 from `invisible-identity`) — the asymmetry is accepted for v1 but should be closed once the cost basis changes (e.g., longer-lived tokens, or once at-rest plaintext exposure becomes a customer signal).
