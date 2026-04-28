---
created: 2026-04-29
status: draft
type: feature
size: M
---

# User Spec: mnemonic-cli (Phase 1 — Programmatic SDK + CLI)

## Что делаем

Шипаем два npm-пакета под scope `@mnemonik-xyz`:

1. **`@mnemonik-xyz/sdk`** — runtime-agnostic JavaScript/TypeScript библиотека для работы с Mnemonic Protocol через hosted MCP-сервер (`mcp.mnemonik.xyz`). Чистый ESM, без node-specific API. Работает в Node 20+, Bun, Deno, современных браузерах, Cloudflare Workers, Chrome-extension контексте. Реализует MCP HTTP-клиент, OAuth 2.1 + PKCE auth-flow (interactive + headless modes), inline COSE_Sign1 подпись через переиспользуемый `@mnemonic/core` WASM-бэкенд, и pluggable `Signer` interface (LocalSigner для Phase 1, TurnkeySigner / WebAuthnSigner — Phase 1.5+).

2. **`@mnemonik-xyz/cli`** — Node-CLI бинарь для пользователей. Тонкая обёртка над SDK: argv-парсинг, persistence keypair'а в `~/.mnemonic/identity.json`, persistence JWT в `~/.mnemonic/token.json`, форматирование вывода (human-readable text по умолчанию, `--json` для пайпа, `--quiet` для CI). Команды: `mnemonic init`, `mnemonic login`, `mnemonic sign`, `mnemonic recall`, `mnemonic verify`, `mnemonic whoami`, `mnemonic prove`.

CLI и Chrome-extension (будущий) — оба consumer'ы одной и той же SDK. Это сознательная архитектурная инвестиция: Phase 1 даёт CLI, но substrate готов для Chrome-extension и agent-фреймворков (LangChain/LangGraph) без переписывания.

## Зачем

**Кросс-провайдерная память для AI-агентов сейчас доступна только тем, кто использует Cursor, VS Code или Claude.ai** — три desktop/web-клиента которые умеют MCP HTTP. Это закрывает целую категорию пользователей:

- Разработчики которые работают только в терминале (vim, emacs, headless ssh) — у них нет MCP-capable редактора
- CI-jobs которые хотят автоматически подписывать build artifacts / commit metadata
- Агентные фреймворки (LangChain, LangGraph, AutoGen, кастомные TS-агенты) которые добавляют tools программно — им проще `import { signMemory } from '@mnemonik-xyz/sdk'` чем городить subprocess-обёртку
- Будущая Chrome-extension которая хочет capture-context из произвольной web-страницы — переиспользует тот же SDK, тот же OAuth flow, тот же COSE backend

CLI расширяет аудиторию протокола за пределы трёх MCP-клиентов. SDK закладывает фундамент под Chrome-extension и agent-интеграции, которые иначе пришлось бы писать с нуля.

Дополнительно для хакатон-демо: возможность показать `npm install -g @mnemonik-xyz/cli && mnemonic sign "..."` за 30 секунд на сцене — это сильный demo-trigger.

## Как должно работать

### Сценарий 1 — Onboarding (`mnemonic init`)

Пользователь устанавливает CLI: `npm install -g @mnemonik-xyz/cli`. Запускает `mnemonic init`. CLI генерит новый Ed25519 keypair (через `@mnemonik-xyz/sdk` → `@mnemonic/core` WASM `generate_keypair`), сохраняет в `~/.mnemonic/identity.json` (формат идентичен webapp's localStorage: `{secret: number[64], pubkey_base58: string}`). Mode 0600. Печатает pubkey + DID. Если файл уже существует — отказывается перезаписать без `--force`.

### Сценарий 2 — Login (`mnemonic login`)

`mnemonic login` запускает interactive OAuth 2.1 + PKCE flow:
1. CLI поднимает локальный HTTP-сервер на свободном порту (loopback 127.0.0.1) для PKCE callback'а.
2. Открывает дефолтный браузер на `https://mc.mnemonik.xyz/oauth/authorize?...&redirect_uri=http://127.0.0.1:<port>/callback`.
3. Браузер делает редирект на `mnemonik.xyz/oauth/consent` (webapp). Пользователь авторизует — webapp подписывает challenge через WASM, отправляет signature на `/oauth/authorize`. Сервер выдаёт authorization code.
4. Браузер редиректит на CLI loopback с кодом, CLI обменивает код на JWT через `/oauth/token`.
5. JWT сохраняется в `~/.mnemonic/token.json` (mode 0600). Печатает "Logged in as <pubkey>".

Headless mode: `mnemonic login --token <jwt>` — пользователь сам передаёт уже-выданный JWT (получен через webapp или другой клиент), CLI не пытается открыть браузер. Для CI / serverless / no-display environments.

### Сценарий 3 — Sign memory (`mnemonic sign`)

```
$ mnemonic sign "Findings from market research: X, Y, Z" --tags=research,demo
attestation_id: 0xabc123
signed: 2026-04-29T10:23:45Z
status: signed (local mode)
```

CLI читает identity.json + token.json, через SDK вызывает `client.signMemory(content, options)`. SDK:
1. Эмбеддит контент через embedding-API hosted сервера (или локально через WASM, если SDK shipsуется с embedder — TBD в техспеке).
2. Собирает canonical CBOR payload через `@mnemonic/core` WASM (байт-в-байт идентично server's `to_canonical_cbor`).
3. Подписывает inline через `LocalSigner` → COSE_Sign1 envelope.
4. POSTs на `/api/sign-callback` с `correlation_id` (получен заранее через `mnemonic_sign_memory` JSON-RPC) и envelope.

Браузер не открывается — у CLI есть ключ, он подписывает локально. Это отличается от webapp flow (где сервер хранит pending bundle и ждёт browser-mediated signing) — CLI юзает свой headless путь через тот же `/api/sign-callback`.

### Сценарий 4 — Recall (`mnemonic recall`)

```
$ mnemonic recall "what did I find about market research"
1 result (top-k=5):
  [score 0.91] 0xabc123 · 2026-04-29 · tags: research,demo
  Findings from market research: X, Y, Z
```

`--json` flag → структурированный JSON для пайпа в `jq`. `--top-k=N` — limit. `--tag=foo` — filter.

### Сценарий 5 — Verify (`mnemonic verify <id>`)

```
$ mnemonic verify 0xabc123
verified: signature valid
storage: local (no on-chain anchor in local mode)
signer: H8x...c4v (you)
```

Exit code `0` если verified, `3` если tampered, `1` если not_found. В full mode добавится: arweave_tx, solana_tx, anchor verification.

### Сценарий 6 — Whoami / prove

```
$ mnemonic whoami
pubkey: H8x...c4v
did: did:sol:H8x...c4v
attestations: 12
storage_mode: local
```

`mnemonic prove [--challenge=hex]` — подписывает challenge локальным ключом, возвращает COSE proof (для `mnemonic_prove_identity` MCP-tool).

### Сценарий 7 — Programmatic SDK use (LangChain / agent / Chrome ext)

```typescript
import { MnemonicClient, LocalSigner, Keypair } from '@mnemonik-xyz/sdk';

const kp = Keypair.fromJSON(JSON.parse(await fs.readFile('~/.mnemonic/identity.json')));
const signer = new LocalSigner(kp);
const client = new MnemonicClient({
  baseUrl: 'https://mc.mnemonik.xyz',
  signer,
  jwt: process.env.MNEMONIC_JWT,  // headless mode
});

const { attestationId } = await client.signMemory('hello', { tags: ['demo'] });
const results = await client.recall('hello', { topK: 5 });
const status = await client.verify(attestationId);  // 'verified' | 'tampered' | 'not_found'
```

## Критерии приёмки

**MUST для Phase 1 (≤5 dev-days, цель — хакатон):**

- [ ] `@mnemonik-xyz/sdk` опубликован на npm (или готов к публикации; реальный publish может быть деплой-таской). Размер пакета ≤500KB (включая WASM).
- [ ] `@mnemonik-xyz/cli` опубликован, `npm install -g @mnemonik-xyz/cli` работает на macOS, Linux, Windows (WSL ок). Node ≥20.
- [ ] **Pure ESM, runtime-agnostic.** Нет `node:*` импортов в `sdk/`. Используем только Web APIs: `fetch`, `crypto.subtle`, `URL`, `TextEncoder`. CLI отдельно может использовать `node:fs`, `node:http`, `node:child_process` (для open browser).
- [ ] **CLI 7 команд:** `init`, `login [--token <jwt>]`, `sign <text> [--tags=a,b]`, `recall <query> [--top-k=5] [--tag=foo]`, `verify <attestation_id>`, `whoami`, `prove [--challenge=hex]`. Все с `--help`.
- [ ] **Output format:** human-readable по дефолту (цвет на TTY, без цвета на pipe или с `--no-color`). `--json` — структурированный JSON в stdout, ошибки в stderr. `--quiet` — только exit code.
- [ ] **Exit codes:** `0` success, `1` user error (no auth, bad args, file not found), `2` server/network error (5xx, refused), `3` integrity failure (verify=tampered), `4` auth error (expired/invalid JWT).
- [ ] **OAuth 2.1 + PKCE interactive flow** работает: `mnemonic login` → браузер открывается → consent на webapp → callback на 127.0.0.1:port → JWT в `~/.mnemonic/token.json`.
- [ ] **OAuth headless mode:** `mnemonic login --token <jwt>` принимает уже-выданный JWT, не открывает браузер.
- [ ] **Inline COSE signing:** `mnemonic sign` подписывает локально через `LocalSigner` + `@mnemonic/core` WASM, POST на `/api/sign-callback`. Никакого browser handoff'а.
- [ ] **`Signer` interface** в SDK: `pubkey: string`, `sign(bytes): Promise<Uint8Array>`. `LocalSigner` имплементирует. SDK API не зависит от concrete signer — Turnkey/WebAuthn drop-in возможен.
- [ ] **Backward compat:** webapp browser-mediated signing flow продолжает работать без изменений (CLI юзает альтернативный path через тот же `/api/sign-callback`).
- [ ] **CI:** unit-тесты SDK (mock fetch, signer interface), integration-тесты CLI против stdio-сервера и/или mock HTTP-сервера. Без сети в CI.
- [ ] **Documentation:** README с quick-start (init → login → sign → recall за 5 строк), JSDoc на public SDK-методы, `mnemonic --help` покрывает все команды.
- [ ] **Демо-сценарий** `npm install -g @mnemonik-xyz/cli && mnemonic init && mnemonic login && mnemonic sign "..."` работает end-to-end.

**Backlog (Phase 1.5+, см. `work/mnemonic-cli/backlog.md`):**

Все следующие — НЕ делаем в Phase 1, но архитектура должна оставаться открытой для них.

- TurnkeySigner / WebAuthnSigner имплементации (абстракция Signer уже готова в Phase 1)
- REPL / TUI mode (`mnemonic repl` — interactive шелл с историей)
- Agent loop в CLI (LLM internal — встроенный chat с tool-calls; но это конкурирует с claude/codex CLIs, низкий приоритет)
- Plugin system (третьи-партии регистрируют свои tools)
- Self-host commands (`mnemonic serve`, `mnemonic init-server` — поднять локальный hosted MCP)
- Multi-account profiles (`--profile work` — несколько keypair'ов в `~/.mnemonic/profiles/`)
- Git-hook integration (`mnemonic precommit-sign` — авто-подпись коммит-метаданных)
- Auto-tagging by current dir / git context
- Browser-mediated signing path в CLI (на случай если CLI работает без локального ключа — например через future Turnkey)
- Public Chrome extension (использует тот же SDK)
- Bundle size optimization (swap WASM на `@noble/curves` если 442KB станет проблемой)

## Ограничения

- **Только hosted mode.** CLI не поддерживает stdio-transport к локальному `mnemonic-mcp`. Если пользователь хочет offline — пусть юзает существующий Rust-бинарь напрямую.
- **One identity per `~/.mnemonic/`.** Multi-profile — backlog.
- **JWT TTL = 1h** (как у webapp). После истечения CLI печатает hint "run `mnemonic login` to refresh". Refresh tokens — backlog.
- **Без LLM-loop.** CLI — thin tool client, не агент. Если нужен chat — пользователь юзает Claude/Cursor/etc и через них вызывает MCP-tools.
- **Pure ESM only.** CommonJS (`require()`) не поддерживается. Это исключает старые Node-проекты на CJS.
- **No browser-only build of SDK initially.** SDK в Phase 1 ориентирован на Node/Bun/Deno/Workers; browser bundle (для Chrome-extension) — добавится в Phase 1.5 (тривиально, тот же ESM работает в браузере + bundle через esbuild).
- **Не клонируем feature parity с claude/codex CLIs.** Те — full chat REPL'ы; мы — thin tool-клиенты.

## Риски

| Риск | Вероятность | Митигация |
|------|-------------|-----------|
| WASM 442KB слишком много для consumer'ов SDK (особенно Chrome-extension с lazy-load) | Средняя | Swap на `@noble/curves` + custom CBOR — в backlog. Public API не меняется. |
| Canonical CBOR в JS (если отказываемся от WASM) расходится с серверным `to_canonical_cbor` побайтно — server отвергает signature | Высокая (если делаем не-WASM путь) | В Phase 1 юзаем WASM (server-identical). Любая будущая JS-реализация валидируется round-trip-тестом против WASM на CI. |
| OAuth callback на loopback ломается за NAT/корпоративным прокси | Низкая | Headless mode `--token` — fallback. Документируем limitation. |
| Bun / Deno / Cloudflare Workers ведут себя по-разному с `crypto.subtle` Ed25519 | Средняя | Тестим на всех 4 runtime'ах в CI. Если subtle не умеет Ed25519 на каком-то runtime — fallback на `@noble/ed25519` (~10KB). |
| Хакатон-судьи не оценивают CLI потому что "не виден на сцене" | Высокая | Demo-flow в спеке: показываем `mnemonic sign` в терминале live, follow up с recall в Claude.ai (та же identity → видны те же данные). Это сильнее чем "ещё одна кнопка" в webapp. |
| Имя организации `@mnemonik-xyz` слишком длинное / не запоминается | Низкая | Можем ребрендить в Phase 2 если занятые `@mnemonik` или `@mnemonic` освободятся. До тех пор — `@mnemonik-xyz` валидно. |

## Технические решения

(полный набор — в tech-spec; здесь только user-facing решения)

- **npm scope: `@mnemonik-xyz`** (org создана пользователем, поскольку `@mnemonik` и `@mnemonic` заняты). Будущая миграция на `@mnemonik` если получится — деплой-таска.
- **Runtime targets: Universal + Bun.** Pure ESM, Web APIs, без `node:*` импортов в SDK. Работает: Node ≥20, Bun, Deno ≥1.40, Cloudflare Workers, modern browsers.
- **COSE backend: `@mnemonic/core` WASM** (через `wasm-pack --target web` build). Идентичен серверному канонизатору. Swap на pure-JS — backlog.
- **Signer interface:** `interface Signer { pubkey: string; sign(bytes: Uint8Array): Promise<Uint8Array> }`. `LocalSigner` для Phase 1; TurnkeySigner / WebAuthnSigner — backlog drop-in.
- **OAuth modes:**
  - Interactive: SDK поднимает loopback, открывает браузер.
  - Headless: SDK получает JWT через конструктор, никаких UI вызовов.
- **Persistence (CLI only):** `~/.mnemonic/identity.json` (mode 0600), `~/.mnemonic/token.json` (mode 0600). SDK сам persistence не делает — это ответственность consumer'а.
- **Output format:** ANSI-цветной на TTY, plain на pipe, `--json` для машинного чтения, `--quiet` для CI. Stderr для логов / прогресса, stdout для результата.

## Тестирование

**Unit (vitest):**
- SDK: mock `fetch`, проверка request shapes (correct OAuth params, correct CBOR encoding, correct COSE envelope).
- SDK: `Signer` interface contract — `LocalSigner.sign()` возвращает 64-byte raw Ed25519 сигнатуру с правильной верификацией.
- CLI: argv-parser, output formatter (TTY vs pipe), exit codes для известных failure modes.

**Integration (vitest + supertest или mock HTTP server):**
- SDK против mock MCP-сервера, который воспроизводит OAuth flow + JSON-RPC tools/list, sign_memory pending response, sign-callback handler.
- CLI через `execa` против mock сервера, проверка stdout/stderr/exit code.

**E2E (минимум 1 сценарий):**
- `mnemonic init` → `mnemonic login --token <pre-issued>` → `mnemonic sign` → `mnemonic recall` против реального hosted сервера (или `STORAGE_MODE=local` self-hosted на CI worker'е). Не блокирующий PR-gate, но есть в release pipeline.

**Cross-runtime sanity:**
- SDK unit-тесты гоняются в Node + Bun в CI (один matrix-step). Deno + Cloudflare Workers — manual smoke перед каждым релизом.

## Как проверить

### Агент проверяет

- `cd packages/sdk && bun test` зелёный
- `cd packages/cli && bun test` зелёный
- `cd packages/sdk && npm pack` — `.tgz` создаётся, размер ≤500KB
- `cd packages/cli && npm pack` — то же
- TypeScript build: `tsc -b` без ошибок
- Lint: `eslint packages/` без ошибок
- WASM-бэкенд: SDK импортирует `@mnemonic/core`, round-trip COSE-encode → server канонизер → match (golden test)

### Пользователь проверяет

После деплоя SDK + CLI на npm registry:

- `npm install -g @mnemonik-xyz/cli` на свежей машине проходит без ошибок
- `mnemonic --help` показывает все 7 команд + `--json`/`--quiet`/`--no-color`
- `mnemonic init` создаёт `~/.mnemonic/identity.json` (mode 0600), печатает pubkey
- `mnemonic login` открывает браузер, после approval'а CLI печатает "Logged in as <pubkey>", `~/.mnemonic/token.json` появляется
- `mnemonic sign "test"` возвращает attestation_id за <2s
- `mnemonic recall "test"` находит только что подписанный attestation
- Та же identity (тот же pubkey) видит те же атестации в webapp (`mnemonik.xyz/install`) и в Claude.ai через MCP — кросс-клиентская портативность
- `mnemonic verify <id>` возвращает `verified`, exit code 0
- `mnemonic verify <some-other-user-id>` возвращает `not_found` (не tampered) — изоляция тенантов работает
- Programmatic SDK use в чужом Node-скрипте: `import { MnemonicClient } from '@mnemonik-xyz/sdk'` работает без ошибок типов в TS
