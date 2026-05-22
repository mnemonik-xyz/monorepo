---
created: 2026-05-21
status: draft
type: feature
size: L  # absorbs work/keypair-sync/, 5 surfaces, 19 tasks/5 waves, marketplace-critical path
priority: P0 (блокирует marketplace shipping; поглощает work/keypair-sync/)
related:
  - work/keypair-sync/user-spec.md (поглощён; закрывается при ship'е этой feature)
  - work/mnemonic-cli/user-spec.md (расширяется — identity.ensure() + keychain)
  - work/mnemonic-core/user-spec.md (расширяется — Rust identity::ensure для mcp-server)
  - work/cursor-vscode-e2e-tests/manual-verify.md (раздел G — Send-to-CLI alignment)
---

# User Spec: Invisible Identity — невидимая загрузка ключевой пары + кросс-поверхностная синхронизация

## Что делаем

Превращаем Ed25519-ключ пользователя в **невидимый сетевой ресурс**, который живёт ровно в одном месте на машине, защищён OS keychain'ом где возможно, и одинаково виден всем поверхностям Mnemonic (Node CLI `@mnemonik-xyz/cli`, Rust stdio MCP-сервер `mnemonic-mcp`, webapp `mnemonik.xyz`, IDE-плагины через JWT).

Поглощает целиком `work/keypair-sync/` — после shipа этой feature `keypair-sync` архивируется в `work/completed/keypair-sync/` с одностраничной заметкой-перенаправлением.

Два слоя поставки:

**Слой 1 — Невидимый bootstrap.**

1. Любая команда CLI или старт mcp-сервера, которая нуждается в identity, **тихо создаёт её при отсутствии** через единый внутренний `identity.ensure()`. Никаких prompt'ов, никаких ошибок "run `mnemonic init` first", никакого ручного редактирования файлов.
2. Секрет хранится в OS keychain (macOS Keychain / Linux Secret Service / Windows Credential Manager) под фиксированными координатами `service = xyz.mnemonik.identity`, `account = default`. Файл `~/.mnemonic/identity.json` остаётся, но содержит только публичные поля + ссылку на keychain entry.
3. На системах без keychain'а (headless Linux без D-Bus, экзотические FreeBSD, Docker-контейнеры) — graceful fallback на старый формат файла с секретом внутри, mode 0600.
4. И Node CLI, и Rust mcp-server читают/пишут **одинаковый keychain entry** и одинаковый `identity.json` — bit-for-bit interop. Если CLI создал identity, mcp-сервер её видит. И наоборот.

**Слой 2 — Кросс-поверхностная синхронизация (поглощено из keypair-sync).**

5. **`mnemonic identity status`** — команда обнаружения drift'а (локально, без сети). Сравнивает локальную identity с JWT.sub из кэшированного `~/.mnemonic/token.json` и сообщает: `synced` / `diverged` / `webapp-unknown` / `no-identity` / `malformed`. Exit code `0` если synced/webapp-unknown, `3` если diverged/malformed, `1` если identity отсутствует.
6. **JWT-baked install deeplinks.** Кнопки "Install in Cursor / VS Code" на webapp вшивают `Authorization: Bearer <jwt>` в сгенерированный `mcp.json` — IDE сразу аутентифицирован тем же keypair'ом что и webapp. Никакого ручного `mnemonic login` или копирования JWT.
7. **Drift-warning prompts.** Webapp "Generate new" показывает модалку с опциями "Send to CLI / Download backup / Cancel" **до** того как новый ключ заменит старый. CLI `mnemonic init --force` напоминает про webapp localStorage drift.
8. **Send-to-CLI / Pull-from-webapp.** Двусторонние flow на one-shot тикетах с TTL 5 минут: либо QR-код / short-code от webapp к CLI, либо `mnemonic identity push-to-webapp` от CLI к webapp. Сырые секреты по сети не идут — передаётся обёрнутый секрет или новый pubkey для linkage (см. tech-spec).

## Зачем

**Два болевых сценария, оба регулярно ломают пользователей сегодня:**

### Onboarding friction блокирует marketplace shipping

Чтобы попасть в VS Code Marketplace / Cursor Extensions Gallery, extension должен пройти review гайдлайн "no required setup on first run". Сейчас наш flow требует от пользователя:

1. Установить CLI (`npm install -g @mnemonik-xyz/cli`)
2. Запустить `mnemonic init` (создаёт `identity.json`)
3. Запустить `mnemonic login` (открывает браузер, делает OAuth, сохраняет JWT)
4. Отдельно настроить `mcp.json` в IDE

Из этих четырёх шагов первые два — чистая церемония вокруг файла. Marketplace reviewer'ы это не пропустят. Невидимый bootstrap превращает шаги 1-3 в один клик "Install" на webapp: identity создаётся при первом MCP-вызове из IDE, JWT уже вшит в install config, пользователь не видит ни одного prompt'а до того момента когда он хочет что-то подписать.

### Drift между поверхностями — повторяющийся footgun

За текущую неделю drift-bug сработал минимум четыре раза:

- **Cursor 0.1.5 sign:** `mnemonic init` создал CLI keypair A, `mnemonic login` минтнул JWT под keypair B (webapp localStorage), при подписи — mismatch, баг #27.
- **IDE OAuth manual paste:** Cursor MCP UI не показывает OAuth flow, пользователь скопировал JWT из CLI вручную, `JWT.sub` = CLI pubkey, webapp подписывает callback с localStorage pubkey — "pending bundle owner mismatch" 403.
- **Webapp test fixtures:** `oauth-flow.spec.ts` всегда генерит свежий keypair на `/install`, локальная CLI identity игнорируется. Корректно для тестов, обнажает структурную проблему.
- **In-memory rollback:** перезапуск сервера почистил OAuth pending state но не JWT-кэш, JWT-ы с предыдущим pubkey стали бесполезны.

Без сторонней синхронизации каждый release post-mortem будет содержать секцию "ещё один drift case". С этой feature drift становится либо невозможным (за счёт shared keychain entry + JWT-baked deeplinks), либо очевидным и one-click reversible (за счёт `identity status` + Send-to-CLI).

## Как должно работать

### Сценарий 1 — Первый `mnemonic sign` на чистой машине

Пользователь ставит CLI и сразу пытается подписать заметку:

```
$ npm install -g @mnemonik-xyz/cli
$ mnemonic sign "First memory on this machine"
mnemonic: identity created did:sol:H8x4...c4v stored in OS keychain
attestation_id: 0xabc123
signed: 2026-05-21T10:23:45Z
status: signed (local mode)
```

Что произошло под капотом:

1. CLI вызвал `identity.ensure()` перед обработкой команды.
2. `~/.mnemonic/identity.json` отсутствовал → сгенерирован новый Ed25519 keypair.
3. Секрет записан в OS keychain (`service=xyz.mnemonik.identity`, `account=default`).
4. На диск записан stub: `{pubkey_base58, did_sol, keychain_ref: "xyz.mnemonik.identity/default"}` + `README.txt` с однострочным описанием папки.
5. Единственная строка в stderr на первом запуске: `mnemonic: identity created did:sol:... stored in OS keychain`.
6. Команда продолжила выполнение — `mnemonic sign` отработал как обычно.

На втором запуске никакого сообщения о создании identity — она уже есть.

### Сценарий 2 — Первый запуск Cursor / Claude Desktop с stdio MCP

Пользователь добавил Mnemonic как MCP-сервер в Cursor через webapp install button (см. сценарий 6) или ручную правку `mcp.json`. При первом MCP-вызове из чата:

1. Cursor запускает `mnemonic-mcp --transport stdio`.
2. Rust mcp-server вызывает `identity::ensure()` в startup-секции `main.rs`.
3. Логика идентична Node CLI: читает `~/.mnemonic/identity.json` → если stub-формат, тянет секрет из keychain → если файла нет вообще, генерит и пишет.
4. Если CLI уже создал identity ранее, mcp-сервер видит её и использует ту же. Если mcp-сервер запустился первым, CLI потом её увидит. **Один keychain entry на машину.**
5. Никаких сообщений в stderr Cursor'у — invisibility включает silent boot для MCP.

### Сценарий 3 — Команда `mnemonic identity status`

```
$ mnemonic identity status
local identity:    H8x4...c4v  (did:sol:H8x4...c4v)
storage:           OS keychain (macOS Keychain)
cached JWT.sub:    H8x4...c4v
status:            synced

```

Если drift обнаружен:

```
$ mnemonic identity status
local identity:    H8x4...c4v
cached JWT.sub:    Kp9z...d2e
status:            DIVERGED — your local key differs from the JWT you used to log in

Suggested actions:
  mnemonic identity pull-from-webapp  # adopt the webapp keypair (after backup)
  mnemonic identity push-to-webapp    # propose this key to your webapp
  mnemonic login                      # re-issue a JWT for your local key
```

Exit code `0` если synced, `3` если diverged. `--json` для пайпа.

### Сценарий 4 — Webapp "Generate new" с drift-warning

Пользователь в webapp идёт на `/settings/keys` и жмёт "Generate new keypair". Перед фактической заменой localStorage показывается модалка:

> **Replace your current keypair?**
> Your JWTs and any IDE / CLI that holds them will stop working for the new pubkey.
> Before continuing:
> - [ ] **Send to CLI** — push current key to your local CLI via QR code
> - [ ] **Download backup JSON** — keep an offline copy
> - [ ] **Cancel**
> - [ ] **Generate anyway** (destructive)

Та же логика на стороне CLI: `mnemonic init --force` спрашивает подтверждение и предупреждает что webapp localStorage останется на старом ключе если его не синхронизировать.

### Сценарий 5 — Send-to-CLI / Pull-from-webapp

**Send-to-CLI (webapp → CLI):**

1. На webapp: меню `Settings → Keys → Send to CLI`. Webapp вызывает `POST /api/cli-bootstrap/issue`. Получает короткий код `ABCD-1234` и QR.
2. На CLI: `mnemonic identity pull-from-webapp ABCD-1234`. CLI обращается к `POST /api/cli-bootstrap/redeem`, получает обёрнутый секрет (wrap-key — короткоживущий x25519 эфемерный ключ, см. tech-spec), разворачивает локально, записывает в keychain + обновляет stub-файл.
3. На обеих поверхностях теперь одинаковый pubkey. JWT тоже валиден без `mnemonic login`.

**Push-to-webapp (CLI → webapp):**

1. На CLI: `mnemonic identity push-to-webapp` сначала тянет статический серверный x25519-pubkey через `GET /api/cli-bootstrap/server-pub`, оборачивает секрет под него, затем POST'ит `/api/cli-bootstrap/issue-from-cli` чтобы получить ticket + short_code. CLI печатает короткий URL `https://mnemonik.xyz/install?pull=<short_code>` и QR-код.
2. Пользователь открывает URL/сканирует QR в браузере где залогинен в webapp. Webapp редимит ticket через `POST /api/cli-bootstrap/redeem`, обёрнутый секрет приходит, разворачивается в браузере и кладётся в localStorage.

Тикеты one-shot, TTL 5 минут, привязаны к user-id (на webapp) и к pubkey (на CLI). Сырые секреты не путешествуют по сети — только обёрнутые x25519.

### Сценарий 6 — Install deeplink из webapp с baked JWT

Залогиненный пользователь идёт на `mnemonik.xyz/install`, видит кнопки `Install in Cursor / Install in VS Code / Install in Claude Desktop`. Клик → webapp генерит `mcp.json` config с уже вшитым `Authorization: Bearer <current-webapp-jwt>`:

```json
{
  "mcpServers": {
    "mnemonic": {
      "url": "https://mc.mnemonik.xyz/mcp",
      "headers": { "Authorization": "Bearer eyJhbGciOi..." }
    }
  }
}
```

Cursor открывается через deeplink `cursor://mcp/install?config=<base64>`, конфиг применяется, MCP-вызовы сразу аутентифицированы. `JWT.sub` равен пользовательскому pubkey'у — никакого drift'а с webapp localStorage не возникает структурно.

Если у пользователя на машине **тоже** есть локальный CLI с identity (через keychain), то drift возможен между IDE-вшитым JWT и локальной CLI identity — это покрывает сценарий 3 (`identity status`).

### Сценарий 7 — Миграция существующего `identity.json`

Пользователь обновился до новой версии CLI. Старый файл:

```json
{"secret":[1,2,3,...,64],"pubkey_base58":"H8x4...c4v"}
```

При первом запуске CLI после обновления:

1. `identity.ensure()` детектит legacy-формат (поле `secret` присутствует).
2. Если OS keychain доступен: секрет копируется в keychain entry, файл переписывается в stub-формат `{pubkey_base58, did_sol, keychain_ref}`. Старый pubkey сохраняется — ключ не меняется.
3. Если keychain недоступен: legacy-формат остаётся как есть, никакой миграции не делается. Однократное сообщение в stderr: `mnemonic: OS keychain unavailable, keeping legacy identity.json`.
4. Mode 0600 валидируется и переустанавливается если нарушен.

Миграция **идемпотентна** и **не теряет ключ** ни в одном из путей. Не делаем `.bak` файла — путь и имя остаются `~/.mnemonic/identity.json`, меняется только внутренняя shape.

## Критерии приёмки

**Слой 1 — Невидимый bootstrap:**

- [ ] `mnemonic sign "x"` на свежей машине (без `~/.mnemonic/`) работает без ошибок, identity создаётся автоматически, секрет уходит в keychain.
- [ ] `mnemonic-mcp --transport stdio` (Rust) на свежей машине стартует без ошибок, использует ту же identity что и Node CLI.
- [ ] На macOS / Linux (gnome-keyring или kwallet) / Windows — keychain entry создаётся и читается обеими языковыми сторонами; содержимое keychain entry **байт-в-байт совпадает** между Rust и Node (golden-byte тест в CI).
- [ ] На headless Linux без D-Bus и в Docker-контейнере без keychain — file-fallback работает, identity сохраняется в legacy-формате `{secret, pubkey_base58}` mode 0600.
- [ ] Существующий `identity.json` legacy-формата мигрируется в stub + keychain entry при первом запуске после апгрейда, без потери ключа.
- [ ] Никаких prompt'ов или интерактивных запросов при первом запуске любой команды. **CLI** на первом создании печатает ровно одну stderr-строку `mnemonic: identity created did:sol:...`; **MCP-сервер (Rust stdio)** при создании identity полностью молчит на stderr (видимость только через структурированный `tracing::info!` в логах). На втором и последующих запусках обе поверхности молчат.
- [ ] `did:sol:` остаётся форматом по умолчанию (без breaking change).
- [ ] `~/.mnemonic/README.txt` пишется на первом запуске с однострочным описанием папки.
- [ ] **Конкурентный bootstrap безопасен:** два процесса (например, CLI и stdio-MCP стартуют одновременно при первом запуске IDE), которые оба видят отсутствующий `identity.json`, не создают два разных ключа. Реализация — atomic-rename через `tempfile + persist` для дискового стуба, идемпотентный keychain `set`, проверка совпадения pubkey после записи. Race выигрывает один, второй читает результат.
- [ ] **Partial-state recovery:** stub-файл без keychain entry → `IdentityRequiresKeystore` с подсказкой `pull-from-webapp`. Keychain entry без stub-файла → пересоздание стуба из keychain (silent recovery). Stub.pubkey ≠ derived(keychain.secret) → громкая ошибка `identity integrity mismatch` (exit 3), запись повреждена, требуется ручное вмешательство — никакого silent picking одной из сторон.

**Слой 2 — Кросс-поверхностная синхронизация:**

- [ ] `mnemonic identity status` возвращает `synced` когда CLI identity и JWT.sub совпадают, `diverged` иначе, exit code 0/3 соответственно.
- [ ] Webapp "Generate new keypair" показывает модалку с опциями Send-to-CLI / Download backup / Cancel перед заменой.
- [ ] CLI `mnemonic init --force` печатает предупреждение, упоминающее как минимум: (a) что webapp localStorage останется на старом ключе, (b) что cached JWT в `token.json` после смены станет невалидным до следующего `mnemonic login`, (c) явный prompt подтверждения с дефолтом "No".
- [ ] `mnemonic identity pull-from-webapp <code>` и `push-to-webapp` работают end-to-end через ticket flow, TTL 5 минут, single redemption. Повторная попытка redemption уже использованного `short_code` возвращает чёткую user-visible ошибку `ticket already redeemed (or expired)` с exit code 3 — никакого silent fail.
- [ ] Webapp install deeplinks (Cursor / VS Code / Claude Desktop) вшивают актуальный JWT пользователя в `mcp.json` config. После клика "Install" в IDE можно вызывать MCP без ручного `mnemonic login`.
- [ ] Четыре pin-точки drift'а из секции "Зачем" покрыты сценарными тестами:
  - **Cursor 0.1.5 sign mismatch** — interop тест Rust mcp-server + Node CLI на shared keychain entry: подпись из IDE проходит против webapp-минтнутого JWT.
  - **IDE OAuth manual paste** — webapp install deeplink flow: после клика "Install" в IDE первый MCP call авторизован без manual JWT-paste; никакого "pending bundle owner mismatch".
  - **Webapp test fixtures игнорировали local CLI identity** — Playwright E2E запускает webapp `/install` с pre-seeded localStorage identity, проверяет что fixture не перетирает её при загрузке страницы.
  - **In-memory rollback инвалидировал JWT** — добавление BootstrapTickets к persistent восстановлению (или явный contract что server restart инвалидирует in-flight тикеты — см. tech-spec); проверка что drift detector корректно ловит этот случай как `diverged`.

**Архивирование `work/keypair-sync/`:**

- [ ] `work/keypair-sync/user-spec.md` перемещён в `work/completed/keypair-sync/user-spec.md`.
- [ ] `work/completed/keypair-sync/MOVED.md` создан с однострочной ссылкой на `work/invisible-identity/`.

**Определение "Shipped":**

- [ ] Wave 5 целиком зелёный — T15 (manual smoke matrix, 5 платформ) подписан, T16/T17 (security + code audits) без unresolved CRITICAL/HIGH, T18 (архивирование) done, T19 (pre-deploy QA gate) зелёный.
- [ ] Feature-ветка `feat/invisible-identity` смерджена в `main` через **один** PR, охватывающий все четыре поверхности (Rust core, mcp-server, Node CLI/SDK, webapp). Атомарный merge — поверхности не релизятся постепенно, чтобы избежать промежуточных состояний где, например, webapp шлёт CLI на endpoint которого ещё нет в опубликованном mcp-server. После merge на main все surface-артефакты (Docker image, npm tarball, webapp build) собираются из одного коммита.
- [ ] **Out of scope для этой feature:** tag release (`v*`), `npm publish` для `@mnemonik-xyz/cli`, фактическая подача в VS Code Marketplace / Cursor Extensions Gallery. Эти шаги становятся возможными после shipа этой feature, но являются отдельными follow-up'ами.

## Ограничения

- **Один identity на машину.** Multi-profile (`--profile work` / `--profile personal`) — backlog. Файл/keychain entry один на `~/.mnemonic/`.
- **Multi-tenant keypair (один пользователь, один CLI, несколько браузеров с разными ключами) — out of scope.** Требует server-side linkage-таблицы, отдельная feature.
- **Hardware-wallet / WebAuthn / Turnkey custodial keypair — out of scope.** Это Phase 2 архитектурно (Tier 3 из старого keypair-sync). Текущая feature только гарантирует что один Ed25519 ключ виден всем поверхностям; не меняет схему хранения секрета вне OS keychain'а.
- **Миграция существующих `local:` synthetic-tx attestations на новый keypair — невозможна без re-signing**, эти строки остаются привязаны к pubkey'у который их подписал.
- **Не меняем существующий OAuth/JWT flow.** JWT всё ещё минтится через webapp OAuth + PKCE; меняется только то что JWT теперь либо вшит в install config (deeplink), либо синхронизирован с локальной identity через ticket.
- **Не меняем формат `~/.mnemonic/token.json`.** Только используем его для drift detection.
- **Не меняем DID-формат по умолчанию.** `did:sol:` остаётся; `--did-format key` — опциональный флаг для будущих фич, в этой feature не реализуется.
- **Browser-side keychain — out of scope.** `BrowserKeyStore` (использование WebCrypto + IndexedDB для webapp) остаётся в `work/marketplace-extensions/` если он там нужен.

## Риски

| Риск | Вероятность | Митигация |
|------|-------------|-----------|
| `@napi-rs/keyring` (Node) и `keyring` crate (Rust) пишут несовместимые форматы в один keychain entry | Средняя | Wave 3 имеет dedicated cross-language interop тесты: Rust пишет → Node читает → byte-equal, и наоборот. Формат внутри keychain — тот же legacy JSON `{secret:number[64], pubkey_base58}` чтобы избежать coordination cost. |
| OS keychain prompt всплывает на каждом MCP-вызове и пугает пользователя | Высокая | На macOS первый доступ закрепляется через `always allow`, в keychain helper'ах Linux — аналогично. Документируем "first access prompts once, subsequent silent". Если prompt'ы упорные на каком-то distro — file-fallback с warning'ом. |
| Headless Linux в CI / Docker без D-Bus делает keychain недоступным → невидимый bootstrap молча валится на file-fallback и пользователь думает что keychain работает | Средняя | `mnemonic identity status` показывает `storage: file (keychain unavailable: <reason>)`. Однократное stderr-сообщение на первой инициализации. |
| Webapp с pre-baked JWT в install config — JWT утекает на github через копипаст `mcp.json` в публичный репо | Средняя | Стандартный warn в UI "this config contains a secret token, don't commit". JWT TTL 1h уже короткий. Long-term — переход на OAuth-в-IDE когда Cursor/VSCode его поддержат. |
| Send-to-CLI тикет перехватывается на пути MITM или через скомпрометированный браузер | Низкая | x25519 wrapping короткоживущим эфемерным ключом, ticket TTL 5 мин, single redemption. Pubkey хеш printedается на CLI и QR — пользователь визуально верифицирует. |
| Cross-platform тестирование keychain не покрывается CI matrix'ом | Высокая | Wave 5 имеет manual smoke matrix на macOS / Ubuntu+gnome-keyring / Windows. Headless Linux в CI покрывает file-fallback. |
| `identity.json` stub-формат vs legacy-формат — рассинхрон между Node и Rust в детекции | Средняя | Tech-spec фиксирует точную shape обоих форматов и алгоритм детекции (`if "secret" in keys then legacy else stub`). Round-trip golden tests. |
| Архивирование `work/keypair-sync/` теряет контекст для будущих читателей | Низкая | `work/completed/keypair-sync/MOVED.md` сохраняется в git'е навсегда; ссылка ведёт сюда. Содержимое user-spec.md keypair-sync целиком интегрировано в эту feature. |
| Конкурентный bootstrap на свежей машине: IDE одновременно стартует stdio MCP и CLI hook'и, оба видят ENOENT и оба генерируют ключи → silent split-brain | Средняя | Atomic-rename pattern для disk-стуба (`tempfile` + `persist`), идемпотентная запись в keychain. Race выигрывает один писатель; второй на retry читает результат. AC закрывает этот случай. |
| Pre-baked JWT в install deeplink выбран вместо ре-OAuth-в-IDE flow | Принятый риск | OAuth-в-IDE требует Cursor/VS Code чтобы прокинуть browser handshake через MCP UI — на момент shipа ни один из них этого не делает (см. risk row выше). Pre-baked JWT — единственный путь к "1-click install без manual login". Митигации: JWT TTL короткий (1h, см. tech-spec), warning в UI "this config contains a secret token, don't commit", переход на OAuth-в-IDE когда клиенты подтянутся. |

## Технические решения (user-facing)

Полный набор архитектурных решений, библиотек, форматов, путей и протокольных деталей — в `tech-spec.md §Decisions` (15 решений) и `tech-spec.md §Data Models`. Здесь зафиксированы только те решения, которые видны или ощутимы конечному пользователю.

- **DID-формат по умолчанию: `did:sol:`** (не меняется, no breaking change). `did:key` — опциональный флаг для будущих фич, в этой feature не реализуется.
- **Single stderr line on creation (CLI only):** `mnemonic: identity created did:sol:H8x...c4v stored in OS keychain` (или `... stored in legacy file (keychain unavailable: <reason>)`). MCP-сервер при создании identity молчит на stderr. Никаких других prompt'ов на любой поверхности.
- **`MNEMONIC_QUIET=1` env var — публичный user-facing override.** Подавляет stderr-строку на первом создании identity. Назначение — headless / CI / Docker / IDE-spawn сценарии, где любой нежданный stderr ломает парсеры или пугает reviewer'ов. Контракт стабильный: следующие минорные версии не сломают переменную и её эффект.
- **`~/.mnemonic/README.txt`:** один абзац, описывает что находится в папке и где искать docs.
- **Сырых секретов в сети нет — никогда.** Send-to-CLI и Push-to-webapp передают только x25519-обёрнутый секрет с TTL 5 минут и single-redemption; протокольные детали обёртки и точные endpoint'ы — в tech-spec.
- **Install deeplink — native URL scheme:** `cursor://mcp/install?config=...` и `vscode:mcp/install?config=...` (платформа-зависимое); Claude Desktop — copy-to-clipboard `mcp.json` (deeplink не поддерживается клиентом). JWT уже вшит в config — никакого ручного `mnemonic login` после клика.

## Тестирование

**Unit (Rust + vitest на Node):**

- `identity::ensure()` создаёт identity если отсутствует, читает существующую без модификации, мигрирует legacy → stub+keychain корректно.
- Keychain mock: пишет → читает → equal bytes.
- File-fallback path: симулировать "keychain unavailable" → identity сохраняется в legacy формате.
- `identity status` детектит synced / diverged / unknown сценарии.
- Ticket flow: issue → redeem → decrypt → match, plus TTL expiry, single-redemption enforcement.

**Cross-language interop (новая категория тестов, Wave 3):**

- Rust пишет identity → Node читает → одинаковый pubkey и подписи.
- Node пишет identity → Rust читает → то же самое.
- Round-trip миграции: legacy file → Node migrates → Rust reads → Rust re-saves → Node reads — bit-identical secret.

**Integration:**

- Node CLI на свежей машине: `mnemonic sign "x"` без ручного init/login работает.
- Rust mcp-server stdio: первый JSON-RPC call после старта на свежей машине проходит, identity создалась.
- `mnemonic identity status` против сценариев synced / diverged.
- Send-to-CLI и Pull-from-webapp end-to-end через mock webapp endpoint.
- Install deeplink → IDE config → MCP call — JWT валиден.

**E2E manual matrix (Wave 5):**

| Platform | Keychain | Bootstrap | Migration | Status |
|----------|----------|-----------|-----------|--------|
| macOS 14 | macOS Keychain | ✓ | ✓ | ✓ |
| Ubuntu 22.04 + gnome-keyring | Secret Service | ✓ | ✓ | ✓ |
| Windows 11 | Credential Manager | ✓ | ✓ | ✓ |
| Docker (no D-Bus) | n/a | file-fallback | n/a | file-fallback |
| Headless SSH (no D-Bus) | n/a | file-fallback | n/a | file-fallback |

**Security review (Wave 5):**

- Key material никогда не логируется и не печатается полностью (только prefix-suffix).
- File permissions 0600 валидируются на старте.
- Ticket secrets не записываются в shell history (CLI принимает их через stdin или env var, не argv).
- JWT в install deeplink не утечёт через URL-encoding в logs (короткий redirect на native deeplink, не через query string на http endpoint).

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. Bootstrap из чистого состояния (Node) | `rm -rf ~/.mnemonic && mnemonic sign "x"` | Команда успешна, `~/.mnemonic/identity.json` появился в stub-формате, keychain entry создан |
| 2. Bootstrap из чистого состояния (Rust) | `rm -rf ~/.mnemonic && mnemonic-mcp --transport stdio` + JSON-RPC `mnemonic_whoami` | Whoami возвращает pubkey, identity создалась без stderr-шума |
| 3. Cross-language: Node-then-Rust | `mnemonic sign "x" && mnemonic-mcp ... mnemonic_whoami` | Тот же pubkey |
| 4. Cross-language: Rust-then-Node | `mnemonic-mcp ... mnemonic_whoami && mnemonic whoami` | Тот же pubkey |
| 5. Migration legacy → stub | Сохранить legacy identity.json, запустить `mnemonic whoami` | Файл переписан в stub-формат, pubkey не изменился, keychain entry создан |
| 6. Drift detection | Подменить JWT в token.json на другой sub, запустить `mnemonic identity status` | Exit code 3, output содержит "DIVERGED" |
| 7. File-fallback | Симулировать недоступный keychain (env var), запустить `mnemonic whoami` | Identity сохраняется в legacy формате, stderr содержит "keychain unavailable" |
| 8. README written | Проверить `~/.mnemonic/README.txt` после первого запуска | Файл существует, содержит описание папки |
| 9. Install deeplink config valid | Открыть webapp `/install`, скачать `mcp.json` | JSON парсится, содержит `Authorization: Bearer <jwt>`, JWT декодируется и `sub` совпадает с webapp pubkey |
| 10. Ticket flow | Issue ticket на webapp, redeem через CLI | После redeem CLI pubkey == webapp pubkey, ticket больше не редимится |

### Пользователь проверяет

- На свежей виртуалке (macOS / Ubuntu / Windows) — `npm install -g @mnemonik-xyz/cli && mnemonic sign "hello"` работает без шагов init и login.
- В Cursor добавить Mnemonic MCP через webapp install button — первый MCP tool call из чата сразу аутентифицирован.
- Сценарий drift: на webapp нажать "Generate new" → увидеть модалку с опциями Send-to-CLI / Backup / Cancel.
- Сценарий sync: на CLI `mnemonic identity push-to-webapp` → отсканировать QR в браузере → webapp localStorage обновился, теперь оба показывают тот же pubkey в `mnemonic whoami` / webapp `/whoami`.
- Сценарий миграции: на машине с уже существующим legacy `identity.json` — после апгрейда CLI старый pubkey сохраняется, файл переписан в stub, keychain содержит секрет. `mnemonic whoami` возвращает тот же pubkey что и до апгрейда.
- Сценарий headless: SSH на сервер без gnome-keyring → `mnemonic sign "x"` работает, identity лежит в legacy-формате файла, никакого crash'а.

