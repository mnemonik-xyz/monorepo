---
created: 2026-05-10
status: draft
type: feature
size: L
---

# User Spec: Chrome Extension `Mnemonik` (Phase 1 — AI-chat capture + Local/Cloud)

## Что делаем

Шипаем публичный Chrome-extension `Mnemonik` (Manifest V3, Chrome Web Store, MIT) — третий MCP-consumer после CLI и webapp. Главная функция: **захват контекста и истории из любого AI-чата в браузере** (ChatGPT, Claude.ai, Gemini, Grok, Perplexity, …) и сохранение его как verifiable memory под одной кросс-клиентской identity. Сохранённый контент далее:

- доступен через `mnemonic_recall` во всех других MCP-клиентах (CLI, webapp, Cursor, Claude Desktop) под той же Ed25519 identity,
- может быть скопирован обратно в новый чат как preloaded context,
- подписан COSE_Sign1 локально в браузере и хранится по выбору пользователя:
  - **Local** — IndexedDB, бесплатно, оффлайн, данные не покидают устройство;
  - **Cloud** — managed Arweave + Solana на Mnemonik AWS-инфраструктуре, платно, синхронизация между устройствами и durable proof.

Auth: **Google OAuth** как primary onboarding, и одновременно — механизм восстановления identity (Ed25519 keypair) на новом устройстве через encrypted key-escrow (passphrase-wrapped blob). Существующий Solana-wallet OAuth остаётся для power-users (CLI, headless).

## Зачем

**Сегодня контекст в AI-чатах эфемерен.** Пользователь работает с ChatGPT / Claude / Gemini, нагенерил полезные промпты, дискуссии, артефакты — а через сутки этого нет ни в поиске, ни в новом чате, ни в другой модели. Window-context съезжает, чат теряется в тред-листе, перенос в другой LLM = copy-paste руками.

Mnemonik закрывает категорию "non-technical knowledge workers" + "vibe-coders на ChatGPT", которые:

- работают преимущественно в браузерных AI-чатах, а не в IDE;
- не поставят CLI и не запустят MCP-сервер;
- хотят **portable cross-LLM memory** — сохранил инсайт в ChatGPT, через час подтянул его обратно в Claude или Gemini одним кликом;
- хотят **proof-of-conversation** — durable, verifiable артефакт чата (для research, compliance, авторства).

Cloud-tier — первый платный SKU с понятным value prop ("sync across devices + durable proof"). Free local-tier даёт zero-friction onboarding и сохраняет privacy-first позиционирование.

## Как должно работать

### Сценарий 1 — Install + first-run onboarding

Пользователь устанавливает `Mnemonik` из Chrome Web Store. При первом запуске popup показывает:

1. Краткий pitch (3 экрана: capture, recall, verify).
2. Выбор tier: **Local (free)** или **Cloud (paid, sign in with Google)**. Local — кнопка "Start", без аккаунта, генерится локальный Ed25519 keypair, сохраняется в `chrome.storage.local`, печатает pubkey + DID в options. Cloud — кнопка "Sign in with Google" → `chrome.identity.launchWebAuthFlow` → выбор Google-аккаунта.
3. Для Cloud: после Google-успеха — экран "Set recovery passphrase" (минимум 12 символов, zxcvbn-проверка ≥3). Объясняем: "passphrase нужен чтобы восстановить identity на другом устройстве; мы не можем его вспомнить за тебя; запиши в password manager". Generate keypair → wrap secret через Argon2id+AES-GCM → upload encrypted blob на сервер (`PUT /api/key-escrow`).
4. Если Google-аккаунт уже привязан к существующей identity (server вернул `existing_pubkey`): экран "Welcome back" → запрос passphrase → fetch blob → decrypt → restore keypair в `chrome.storage.local`. Wrong passphrase: 5 попыток → 24h cooldown.

### Сценарий 2 — Capture AI chat (главная функция)

Пользователь открыл `chatgpt.com` (или `claude.ai`, `gemini.google.com`, `x.com/i/grok`, `perplexity.ai`, …). Extension content-script инжектит floating action button (FAB) в правый-нижний угол страницы (опционально hide-able) и пункт в context-menu "Save to Mnemonik".

**Save selection** — пользователь выделил кусок ответа модели (или своего промпта), правый клик → "Save selection to Mnemonik" (или горячая клавиша `Cmd/Ctrl+Shift+M`). Popup всплывает, prefilled tags из URL (`source:chatgpt`, `model:gpt-5`, `chat:abc123`) + первая строка как title, кнопка "Sign". Подпись локально, IndexedDB save (Local) или upload (Cloud), toast "Saved · attestation 0xabc…".

**Save whole conversation** — клик по FAB → "Save this chat". Content-script экстрактит всю текущую беседу через per-platform DOM-adapter, нормализует в structured markdown (JSONL of `{role, content, ts}` в payload + human-readable .md в content). Popup показывает preview + tag editor + storage-tier confirmation. Sign → Save.

**Auto-capture (opt-in, options page)** — для определённого домена включить background watcher: каждый новый assistant-response в чате автоматически добавляется к "draft attestation"; в конце сессии (или по кнопке) — single sign covering the full conversation. Off по дефолту.

### Сценарий 3 — Recall context back into a chat

В popup или на любой странице с открытым AI-чатом: глобальная горячая клавиша `Cmd/Ctrl+Shift+R` → mini-search overlay → пользователь печатает запрос → top-5 семантически близких memories → выбирает → опции:

- **Copy to clipboard** (markdown с ссылкой на attestation_id),
- **Insert into chat input** (если adapter знает где input у текущего домена) — вставляет в поле ChatGPT/Claude/etc.,
- **Open in popup** (полный текст + verify badge + источник).

### Сценарий 4 — Verify

Popup → "Verify" tab → paste attestation_id или drop файл с COSE-bundle → verified / tampered / not_found, с метаданными (signer, ts, source-platform).

### Сценарий 5 — Switch storage mode

Options page → "Storage" → переключатель Local/Cloud. Local→Cloud: явный prompt "upload N existing attestations to cloud?" → resumable очередь. Cloud→Local: "export and disconnect" → скачивает .zip с COSE-bundles + удаляет escrow blob по запросу.

### Сценарий 6 — Restore identity on new device

Установил extension на втором ноуте → Sign in with Google (тот же аккаунт) → server: `existing_pubkey: "H8x...c4v"` → popup показывает "Welcome back, identity H8x...c4v detected" → ввести recovery passphrase → fetch blob (`GET /api/key-escrow`) → Argon2id-derive key → AES-GCM-unwrap → keypair восстановлен в `chrome.storage.local` → cloud-history синхронизируется в IndexedDB. Если passphrase утерян: либо "start fresh" (новый keypair, теряется доступ к подписанной cloud-истории под старым ключом — данные читать можно, подписать новые от старого имени нельзя), либо импорт keypair-файла из webapp/CLI.

### Сценарий 7 — Rotate passphrase

Options → Security → "Rotate passphrase" → ввести old + new → re-encrypt blob → `PUT /api/key-escrow`.

### Сценарий 8 — Programmatic / cross-client portability

Та же identity (тот же pubkey) видит те же атестации в webapp, CLI (`mnemonic recall`), Claude.ai через MCP, Cursor. Ничего платформо-специфичного в payload — markdown + structured chat JSON универсальны.

## Критерии приёмки

**MUST для Phase 1:**

- [ ] Manifest V3, проходит Chrome Web Store review (no remote code, declared permissions justified).
- [ ] **Chat-adapter framework** + рабочие adapter'ы для ChatGPT (`chatgpt.com`), Claude.ai (`claude.ai`), Gemini (`gemini.google.com`). Grok / Perplexity — backlog if time.
- [ ] **Save selection** работает на любом сайте (не только AI-чатах) через context-menu + горячую клавишу.
- [ ] **Save whole conversation** работает на трёх supported платформах: экстрактится правильный turn-order, role labels, code blocks сохраняются как fenced markdown.
- [ ] **Recall overlay** (`Cmd/Ctrl+Shift+R`) работает: top-k семантический поиск, copy/insert/open actions.
- [ ] **Local mode** полностью оффлайн: zero network в DevTools после первого page-load. IndexedDB versioned schema. Synthetic `local:` tx ids идентичны server-`local`.
- [ ] **Cloud mode**: Google sign-in → set passphrase → keypair generated + escrowed → save chat → server анкорит на Arweave+Solana → recall на втором устройстве после restore возвращает тот же memory.
- [ ] **Restore flow** работает end-to-end: fresh extension install → Google sign-in (existing user) → enter passphrase → identity restored → cloud-history pulled.
- [ ] **Wrong-passphrase rate-limit**: 5 fetches / 24h / `google_sub`, server-enforced.
- [ ] **Bundle budget**: popup initial JS ≤50KB, total package ≤2MB (excl. lazy-loaded embedder model ~25MB, cached after first download).
- [ ] **Full sign pipeline ≤2s** на M1 после warm WASM (warm: model loaded, WASM instantiated). Cold-start ≤8s.
- [ ] **Accessibility**: keyboard navigation, ARIA labels, screen-reader friendly popup.
- [ ] **Privacy policy** опубликован, явно говорит: local-mode = data stays on device, cloud-mode = encrypted-at-rest на AWS, escrow blob не E2EE для контента (только для secret key).
- [ ] **CI**: vitest unit + Playwright E2E с загруженным extension; на supported платформах — smoke tests против записанных HAR.

**Phase 1.5+ (backlog, не блокирующий):**

- Auto-capture mode (per-domain watcher).
- Grok / Perplexity / Poe / OpenRouter chat adapter'ы.
- "Insert into chat input" для всех supported платформ (Phase 1 — только copy-to-clipboard).
- Firefox / Safari порты.
- WebAuthn (passkey) wrap взамен passphrase.
- Team / shared memories.
- E2EE attestation content (зашифровано до upload).
- Custom embedder model selection.

## Ограничения

- **Chromium only Phase 1** (Chrome, Edge, Brave, Arc). Firefox/Safari — backlog.
- **Embedder fixed**: `Xenova/all-MiniLM-L6-v2` (384 dim, ~25MB). User не выбирает модель.
- **Adapter brittleness**: AI-chat UIs меняются часто; adapters могут сломаться, нужен update path. Версия adapters bundled с extension; auto-update через Chrome Web Store releases.
- **No E2EE для attestation content в cloud-tier MVP**. Encrypted at rest + in transit, but server can technically read. Документировано. Phase 2 добавит client-side encryption.
- **Recovery passphrase не recoverable**: lost passphrase = lost ability to restore on new device без manual keypair file.
- **Auto-capture off by default** — privacy. User должен explicitly enable per domain.
- **Cloud-tier requires online** для sign-callback; offline создаёт queue, синкается при reconnect.
- **Identity = одна Ed25519 keypair** на user across CLI/webapp/extension. Multi-account — backlog.

## Риски

| Риск | Вероятность | Митигация |
|------|-------------|-----------|
| ChatGPT/Claude/Gemini DOM меняется → adapter ломается | Высокая | Adapter'ы изолированы за `ChatAdapter` интерфейсом; CI smoke runs против записанных HAR; bug-report кнопка в popup отправляет broken-adapter telemetry (opt-in). |
| Chrome Web Store review reject (broad host_permissions, "captures user data") | Средняя | Минимизируем permissions: `activeTab` + специфичные host_permissions per supported domain (не `<all_urls>`); explicit user opt-in для capture; privacy policy с прямым text. |
| WASM 442KB + 25MB embedder = slow first-sign UX | Средняя | Lazy-load model в Web Worker при первом sign; progress UI; кэш в Cache API после первого скачивания. |
| Утерянный passphrase = поддержка-тикеты "верните мою identity" | Высокая | Жёсткий messaging при онбординге; password-manager prompt; "import keypair from webapp/CLI" как fallback. |
| Google OAuth scope creep (запросим больше чем openid+email) | Низкая | Только `openid email profile`, никаких Drive/Gmail scopes. |
| IndexedDB quota eviction при долгом неиспользовании | Низкая | `navigator.storage.persist()` request на onboarding; UI warning при approaching quota. |
| Embedder vector dim ≠ серверной → recall в cross-client сценариях возвращает не то | Высокая если разъедутся | T5 валидирует round-trip против golden-fixtures из `core/`; CI gate. |

## Технические решения (user-facing)

- **Default tier: Local.** Cloud — opt-in через Google sign-in. Это сохраняет privacy-first messaging и снижает onboarding friction.
- **Cloud requires Google sign-in + recovery passphrase.** Без passphrase нет escrow — но user может opt-out из escrow (тогда identity привязана к этому одному устройству, на втором нужен manual import).
- **One identity per user** across all clients. Switching modes не меняет keypair.
- **Adapter framework** документирован — можем принимать community contributions для новых платформ.
- **Floating action button** опционален (hide через options или per-domain).
- **Hotkeys customizable** (Chrome's `commands` API).
- **Telemetry**: opt-in only (broken-adapter reports, anonymous). Default off.

## Тестирование

**Unit (vitest):**
- IndexedDB store CRUD (mock IDBFactory).
- Embedder round-trip с deterministic seed.
- WASM signer parity vs server golden-fixtures.
- Argon2id + AES-GCM wrap/unwrap round-trip.
- Each chat adapter: parse фиксированный HTML snapshot → expected JSONL.

**Component (Playwright):**
- Popup: capture, recall, verify, options flows.
- Onboarding: Google → passphrase → keypair → cloud upload (mocked server).
- Restore: fresh storage → Google → passphrase → keypair restored.

**E2E (Playwright with `--load-extension=dist/`):**
- chat capture on ChatGPT (against recorded HAR fixture).
- chat capture on Claude.ai.
- chat capture on Gemini.
- recall overlay on arbitrary page.
- mode switch local→cloud.
- restore on second profile.

**Server (Rust, in `mcp/tests/`):**
- `oauth_google.rs` — mock Google JWKS, full PKCE round-trip.
- `key_escrow.rs` — PUT/GET/DELETE, rate-limit enforcement.

**Security:**
- Manual review: CSP (no `unsafe-eval`, no remote scripts).
- `web-ext lint dist/` clean.
- Argon2id parameters meet OWASP 2026 minimums (`memory_cost ≥ 64MiB, time_cost ≥ 3`).

## Как проверить

### Агент проверяет

- `cargo test --workspace --no-fail-fast` зелёный (новые `oauth_google.rs`, `key_escrow.rs` в составе).
- `pnpm -C packages/extension test` зелёный.
- `pnpm -C packages/extension build` создаёт `dist/` с валидным MV3 manifest.
- `web-ext lint dist/` без warnings.
- Bundle size budget enforcement в CI (popup ≤50KB initial, total ≤2MB).
- Round-trip COSE/CBOR test против golden-fixtures из `core/`.

### Пользователь проверяет

- Установка распакованного extension в Chrome → onboarding → выбор Local → save selection на любом сайте → recall находит.
- Выбор Cloud → Google sign-in → set passphrase → save full chat в ChatGPT → в webapp под той же identity видна та же атестация.
- На втором профиле/устройстве: install → Google sign-in → enter passphrase → identity restored → cloud-history появляется в popup.
- Wrong passphrase 5 раз подряд → 6-я попытка блокируется на 24h.
- Toggle "auto-capture" для chatgpt.com → каждый новый assistant-message добавляется в draft → "Finalize" → один attestation на всю беседу.
- Recall overlay (`Cmd/Ctrl+Shift+R`) на странице Claude.ai → найти memory, скопированный из ChatGPT неделю назад.
