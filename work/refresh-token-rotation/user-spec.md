---
created: 2026-06-06
status: draft
type: feature
size: M
priority: P0
related:
  - work/stateless-auth-rearch/README.md (параллельный stub: long-term per-request signing; альтернативный путь, отложен)
  - work/binary-mode-cleanup/README.md (parking lot: убираем HTTP-local mode целиком, только participate на hosted; отдельная фича)
  - work/chrome-extension-client-side-storage/README.md (parking lot: миграция extension'а на IndexedDB; unblocks binary-mode-cleanup)
  - work/completed/modes-user-choice/user-spec.md (invariant «Личная память бесплатна всегда» — не трогаем)
  - docs/WHITEPAPER.md §5.7 Protocol Economics
---

# User Spec: OAuth refresh tokens — сессия не умирает мид-разговора

## Что делаем

Добавляем стандартный OAuth 2.1 refresh-token flow к `/oauth/token` сервера
mnemonik-mcp. Access-token остаётся 1ч (без изменений). Новый refresh-token
живёт 1 год, **rolling** — каждое использование выпускает новый refresh
и инвалидирует старый. В пределах короткого «reuse-interval» (30 секунд)
повторное предъявление того же refresh-токена возвращает уже-выданную
descendant-пару — это защищает параллельные клиенты от kill-the-session
race. Клиенты OAuth 2.1 (Cursor, VS Code Copilot, Claude Desktop)
подхватывают это автоматически и тихо ротируют без участия пользователя.

## Зачем

Сегодня JWT-access-token живёт 1 час. Через час сессия в Claude Desktop
умирает: каждый последующий `tools/call` к auth-гейтнутому тулу
возвращает `-32001 unauthorized: invalid JWT: ExpiredSignature`. Внутри
сессии восстановить токен нельзя — харнес прицепляет пару
`authenticate`/`complete_authentication` только при коннекте, и наш
сервер на момент коннекта рапортует валидный токен, так что пара не
цепляется.

Воспроизводилось в живой сессии 2026-06-06: пользователь хотел сохранить
контекст разговора через `mnemonic_sign_memory`, получил unauthorized,
не смог продолжить и пришлось писать workaround в `HANDOFF.md` в репо.
Такая поломка отравляет любую сессию длиннее часа.

Stripe MCP — hosted OAuth MCP-сервер сопоставимого профиля — этой
проблемы не имеет. Их recipe: 1ч access + 1г rolling refresh. Юзер
логинится **раз в год максимум**, при условии что в течение года была
хотя бы одна активная сессия. Клиенты OAuth 2.1 ротируют refresh-токены
по стандарту — это не их изобретение, это RFC.

После фикса:
- Любой OAuth 2.1-совместимый MCP-хост сохраняет сессию пока юзер активен.
- Юзер видит OAuth-страницу только при первом подключении или после
  очень длительного перерыва (>1г).
- Никаких новых MCP-тулов, никаких изменений в model/visibility/pricing.

## Как должно работать

1. **Первичный логин (как сегодня):** юзер открывает MCP-хост, тот идёт
   на `/oauth/authorize`, юзер подписывает challenge в webapp consent
   page, MCP-хост получает `code`, обменивает на `/oauth/token` с
   `grant_type=authorization_code`.
2. **Ответ `/oauth/token` (NEW):** возвращает трёх-полевой JSON
   вместо двух:
   ```json
   {
     "access_token": "eyJ...",
     "token_type": "Bearer",
     "expires_in": 3600,
     "refresh_token": "rt_..."
   }
   ```
   Поддерживаются оба content-type'а (form-encoded для Cursor/VS Code,
   JSON для Claude Desktop) симметрично существующему authorization_code
   flow.
3. **MCP-хост работает как обычно** в течение часа. На каждый запрос —
   Bearer access-token, сервер валидирует, отдаёт результат.
4. **Access-token протух (через час):** хост получает 401 с
   `WWW-Authenticate: Bearer ... error="invalid_token"`. Это его сигнал
   ротировать.
5. **Тихая ротация (NEW):** хост шлёт `POST /oauth/token` с
   `grant_type=refresh_token` + сохранённый `refresh_token`. Сервер в
   одной BEGIN IMMEDIATE транзакции:
   - находит refresh-токен по blake3-хэшу,
   - проверяет `expires_at > now AND revoked = 0`,
   - помечает старый как revoked + `rotated_to = <new_hash>` (для
     детекта реплея),
   - минтит новый access + новый refresh + расширяет `expires_at` на
     1г от now (rolling),
   - возвращает оба клиенту.
6. **Reuse-interval защита от concurrent-race (NEW):** если в течение
   30 секунд после ротации тот же `rt_X` приходит снова — сервер
   возвращает **тот же descendant pair**, что был выдан первому
   запросу (идемпотентный путь). Это покрывает кейс «два параллельных
   401 → два параллельных refresh-grant» без kill-the-session.
   **Только** предъявление revoked-refresh **после** окна → family-revoke.
7. **Хост подставляет** новый access в Authorization header и
   повторяет упавший запрос. Пользователь ничего не видит.
8. **Юзер активен ≤ 1 год:** каждая ротация выдвигает срок refresh на
   ещё год — юзер живёт бесконечно.
9. **Юзер пассивен > 1 год:** refresh-token expired — хост получает
   `400 invalid_grant`, ведёт юзера на полный OAuth-логин. Это норма
   и редко.

**Защита от replay:** старый refresh-токен, использованный **после**
reuse-interval окна, вызывает revocation **всего семейства** (chain
`rotated_to` + `family_id`). Это стандартный паттерн OAuth 2.1 §6.1.

## Критерии приёмки

- [ ] AC1 — `/oauth/token` с `grant_type=authorization_code` возвращает
  JSON содержащий `refresh_token` (непустая строка) в дополнение к
  существующим `access_token`, `token_type`, `expires_in`.
- [ ] AC2 — `/oauth/token` с `grant_type=refresh_token` и валидным
  `refresh_token` возвращает 200 с **новым** `access_token` (отличается
  от старого) и **новым** `refresh_token` (отличается от того, что
  прислали).
- [ ] AC3 — После reuse-interval (30s) старый `refresh_token`
  использовать нельзя: `/oauth/token` с `grant_type=refresh_token`+
  `старый_токен` возвращает `400 invalid_grant`.
- [ ] AC4 — Replay detection после reuse-interval: попытка использовать
  revoked refresh-token вне окна инвалидирует **всю цепочку ротаций**
  (родитель + все потомки по `family_id`). После этого даже самый свежий
  refresh из цепочки возвращает `400 invalid_grant` — юзер должен пройти
  полный OAuth.
- [ ] AC5 — Истёкший refresh-token (`expires_at <= now`) возвращает
  `400 invalid_grant`. Новые токены не выпускаются, family-revoke
  не триггерится (истечение ≠ атака).
- [ ] AC6 — Refresh-token живёт 1 год от момента **последней
  ротации** (rolling). Каждое успешное использование двигает
  `expires_at` ровно на `now + 1y`. Тест проверяет: после ротации
  `expires_at > expires_at_before` строго (а не просто «новый токен
  отличается»).
- [ ] AC7 — Discovery metadata `/.well-known/oauth-authorization-server`
  содержит `"refresh_token"` в массиве `grant_types_supported`.
- [ ] AC8 — Существующее поведение access-token не сломано: 1ч TTL,
  HS256, та же `Claims` структура с `sub`, `iat`, `exp`, `jti`,
  `google_sub`. Старые auth-гейтнутые тулы продолжают возвращать `-32001`
  на истёкшем access-токене.
- [ ] AC9 — Анонимный `mnemonic_recall` (регрессионный якорь) продолжает
  работать без изменений — `mcp/tests/anonymous_recall.rs` не сломан.
- [ ] AC10 — **Dual content-type parity**: refresh-grant работает
  идентично в `application/x-www-form-urlencoded` (как у Cursor /
  VS Code) и в `application/json` (как у Claude Desktop). Существующий
  authorization_code flow тоже работает в обоих форматах сегодня — фича
  сохраняет этот инвариант.
- [ ] AC11 — **Wire-format back-compat**: клиент, который игнорирует
  поле `refresh_token` в ответе и продолжает использовать только
  `access_token` (как наши легаси-клиенты до этой фичи), работает без
  изменений. Поле — additive, никаких breaking-change.
- [ ] AC12 — **Concurrent rotation внутри reuse-interval**: два запроса
  ротации с одним и тем же `rt_X` в течение 30s после первой ротации
  оба получают одну и ту же descendant pair (идемпотентность через
  reuse-interval lookup). Семья **не** revoked. Тест запускает 2
  параллельных refresh-grant'а и проверяет, что обоим вернулся
  один и тот же новый access+refresh.
- [ ] AC13 — **Malformed refresh-grant**: `grant_type=refresh_token` без
  поля `refresh_token` (или с пустой строкой) возвращает
  `400 invalid_request` (не `invalid_grant` — это не неверный токен,
  это неверный запрос).

## Ограничения

- Не меняем формат access-token'а (Claims, HS256, 1ч TTL).
- Не меняем существующий `authorization_code` flow (PKCE, redirect URI
  validation, code TTL 60s — всё как сегодня).
- Не меняем browser-mediated signing flow для `sign_memory` (отдельный
  слой, COSE_Sign1 на байтах памяти).
- Не меняем `mode` / `visibility` модель (это `work/binary-mode-cleanup/`).
- Не реализуем `/oauth/revoke` endpoint в первой итерации — добавляем
  только если он нужен MCP-хосту для logout (TBD по результатам smoke).
- Refresh-token — **opaque random string** (32 байта base64url), не JWT.
- Refresh-grant поддерживает **оба** content-type'а (form-encoded и
  JSON) симметрично существующему authorization_code endpoint'у.
  Серверный dispatch в `token_handler` уже это делает.
- Архитектурное правило: OAuth/payment/pricing живут в `mcp/`, не в
  `core/`. Refresh-token storage идёт в `mcp/src/oauth/refresh.rs` по
  паттерну `migrate_key_escrow_blobs` (`escrow.rs:113-133`).
- Refresh-token never crosses process boundary as plaintext — храним
  только blake3-хэш в БД, плейн отдаём клиенту один раз. Blake3 — по
  прецеденту `payment.rs:737-744`.
- Stdio transport не трогаем — там JWT-пути нет.
- **CLI cache `~/.mnemonic/token.json` НЕ расширяется** в V1 — сегодня
  он хранит только access JWT (`cache_minted_token`). Добавление
  refresh-token slot'а требует синхронных правок в client-side state
  machine агент-native-distribution. Open `work/cli-refresh-token-support/`
  follow-up если CLI-юзеры запросят.
- **Observability** в V1 ограничена существующими `tracing` логами:
  лог при успешной ротации (sub, family_id), лог при family-revoke,
  лог при reuse-interval hit. Prometheus-метрики (счётчики ротаций,
  частота replay-detect) — отдельный follow-up; не блокирует ship.
- Per-request signing (true stateless auth) **не делаем** — отложено
  в `work/stateless-auth-rearch/` как long-term direction.

## Риски

- **R1 — Claude Desktop может не поддерживать refresh-токены.**
  Эмпирически неизвестно. Cursor / VS Code Copilot ротируют (OAuth 2.1
  standard); Claude Desktop — гипотеза до проверки.
  **Mitigation (pre-ship, обязательно):** одна из двух дешёвых проверок
  ДО ship:
  - **Option A**: захватить HTTP-трейс Claude Desktop ↔ Stripe MCP
    (https://mcp.stripe.com — у них refresh-tokens работают), посмотреть
    делает ли Claude Desktop `grant_type=refresh_token` POST на их
    `/oauth/token`. Если да — наш фикс сработает по тому же протоколу.
  - **Option B**: задеплоить v0.2.5-dev с `JWT_TTL_SECS=60`, подключить
    Claude Desktop, ждать 2 минуты, наблюдать поведение. Если ротирует —
    подтверждено. Если нет — `refresh_token` для Claude Desktop ничего
    не даёт, открываем эскалацию (Anthropic ticket / переключаемся на
    stateless-auth-rearch путь).

  Без одной из этих проверок ДО ship — это та же ставка на «потом
  узнаем», которая угробила два предыдущих пивота этого спека.
- **R2 — Replay-attack window.** Если refresh-token утёк (логи, клиент)
  — атакующий может ротировать один раз и получить валидную пару.
  Standard OAuth 2.1 risk. **Mitigation:** revoke-family при детекте
  использования revoked-токена **вне** reuse-interval (AC4), HTTPS-only,
  Token Binding мы не делаем.
- **R3 — Refresh-token storage compromise.** Если БД с хэшами утечёт —
  плейн-токены не реверсятся (blake3), но атакующий получает список
  активных `sub`. **Mitigation:** blake3+salt; БД не должна попадать в
  бэкапы без шифрования. Это операционное требование, не код.
- **R4 — Clock skew между сервером и клиентом.** Хост может ротировать
  слишком рано (думает access протух, на сервере он ещё жив). Не
  страшно — refresh-grant всегда работает пока refresh валиден; просто
  лишний RPC. **Mitigation:** нет, это OK по стандарту.
- **R5 — DB migration.** Добавляется таблица. Должна быть идемпотентна
  и совместима с rolling deploy. **Mitigation:** `CREATE TABLE IF NOT
  EXISTS`, никаких ALTER на существующих таблицах. Миграция в
  `mcp/src/oauth/refresh.rs::migrate_refresh_tokens` по паттерну
  `escrow.rs::migrate_key_escrow_blobs`.
- **R6 — DB write failure during rotation.** SQLite может сфейлить
  (диск, lock contention timeout, etc.) во время BEGIN IMMEDIATE
  транзакции. **Mitigation:** возвращаем `500 internal_error` (не 400 —
  это не вина клиента), клиент стандартно ретраит (для рефреш-грантов
  это безопасно — операция идемпотентна в течение reuse-interval).
  Лог события + метрика для observability follow-up.
- **R7 — Глобальный logout невозможен** (D16). Поскольку access-token
  остаётся JWT (D7), у нас нет способа «log out everywhere» через
  revoke-list — только смена HMAC-секрета (инвалидирует всех). Это
  ограничение существует уже сегодня; refresh-tokens его не исправляют
  (могут revoke только refresh-family, access JWT ходит до своего TTL).
  Если потребуется глобальный logout как фича — отдельный feature,
  скорее всего совмещённый с переходом на opaque access-token'ы.

## Технические решения

- **D1 (Refresh-token format):** Opaque 32-byte random string,
  base64url-encoded для transport. Сервер хранит **blake3(salt+token)**
  хэш (не sha256 — по прецеденту `payment.rs:737-744` в нашем коде
  блейк3 уже используется для API-ключей, держим унифицированный
  hash-primitive). Плейн возвращается клиенту один раз, никогда не
  логируется.
- **D2 (TTL — Stripe-precedent с обоснованием для memory):** Access 1ч
  (без изменений), refresh 1 год rolling. Stripe тоже использует 1y;
  для нашего memory-protocol профиля это оправдано тем, что один
  attestation — это одна вставленная память (не финансовая транзакция),
  риск «продлённой сессии после компрометации» материализуется в
  потенциально лишних записях, но не в потере денег. 90 дней — тоже
  разумная альтернатива; мы оставляем 1y, потому что (а) Stripe-копи,
  (б) минимизирует частоту OAuth-страниц для активных юзеров. Параметр
  легко изменить в одной константе если опыт покажет иное.
- **D3 (Rotation discipline — OAuth 2.1 §6.1):** Каждый refresh-grant
  выпускает новый refresh и помечает старый `revoked=1` +
  `rotated_to=<new_hash>`. Detects replay если старый предъявляется
  после reuse-interval.
- **D4 (Replay revokes семью):** При предъявлении revoked-refresh
  **после** reuse-interval — revokе всех refresh'ей в семействе по
  `family_id`. Forces full re-OAuth. Стандарт OAuth 2.1 §6.1.
- **D5 (Storage location):** Refresh-token таблица в `mcp/`, не в
  `core/`. Архитектурное правило CLAUDE.md.
- **D6 (No `/oauth/revoke` v1):** Нет UX-сценария требующего
  programmatic logout сегодня. Add if MCP host requests.
- **D7 (Access-token формат не меняется):** Claims, HS256, 1ч TTL.
  Middleware-side surface unchanged. **Trade-off (operational):**
  отсутствует «log out everywhere» capability — см. R7. Принимаем
  ограничение в V1 как стоимость не-меняющего-существующее решения.
- **D8 (Discovery metadata):** Добавить `"refresh_token"` в
  `grant_types_supported` в `/.well-known/oauth-authorization-server`.
- **D9 (No Token Binding):** Out of scope. HTTPS-only — это binding.
- **D10 (OAuthState gets DB handle):** Расширяем `OAuthState` чтобы
  принимал `Arc<Mutex<rusqlite::Connection>>` на тот же файл что
  использует `McpState.store`. Не отдельный DB-файл. Operationally
  проще (один файл, один бэкап). `McpState::new` вызывает
  `OAuthState::new(store_arc.clone())`. Миграция в
  `mcp/src/oauth/refresh.rs::migrate_refresh_tokens(&conn)` следует
  паттерну `escrow.rs::migrate_key_escrow_blobs` —
  идемпотентный `CREATE TABLE IF NOT EXISTS`, регистрируется в
  `mcp/src/main.rs:499-505`.
- **D11 (Atomic rotation):** Ротация в одной `BEGIN IMMEDIATE`
  транзакции: SELECT-with-write-lock → check `revoked=0 AND expires_at
  > now` → UPDATE old (revoked=1, rotated_to=<new>) → INSERT new. Если
  конкурентный refresh-grant — второй блокируется на immediate-lock,
  потом видит revoked=1 и попадает в reuse-interval lookup (D13).
  Pattern from `payment.rs:478-505`.
- **D12 (Eviction):** Hourly background sweep
  (`Duration::from_secs(3600)`) по паттерну
  `confirmation_token::start_evictor`. Plus opportunistic cleanup:
  при каждой успешной ротации в той же транзакции —
  `DELETE FROM refresh_tokens WHERE family_id=? AND expires_at < now`
  чтобы держать таблицу маленькой.
- **D13 (Reuse-interval — Auth0/Okta pattern):** 30-секундное окно
  после ротации. Если тот же `rt_X` приходит ещё раз внутри окна —
  сервер lookup'ит запись с `rotated_to != NULL AND revoked_at + 30s
  > now`, и **возвращает уже-выданную descendant pair** (не выпускает
  третий токен, не revokе семьи). Solves concurrent-401 race без
  компрометации replay-detect: только вне окна reused `rt_X` →
  family-revoke. Auth0 называет это «refresh token reuse interval».
- **D14 (Dual content-type parity):** Новая refresh-ветка в
  `token_handler` парсит body после dispatch'а на `application/json`
  vs `application/x-www-form-urlencoded` (тот же путь что для
  authorization_code, см. `oauth/mod.rs:982-1078`). Никаких новых
  парсинговых веток.
- **D15 (CLI cache out of scope):** `cache_minted_token` в `oauth/mod.rs:
  1065-1071` пишет только access JWT в `~/.mnemonic/token.json`.
  Refresh-token slot не добавляется в V1 — это работа в
  client-side state machine агент-native-distribution. Open
  `work/cli-refresh-token-support/` follow-up если запросят.
- **D16 (No global logout v1):** Принимаем что V1 не даёт «log out
  everywhere» (R7). Это уже сегодня так; refresh-tokens не лечат, но
  и не ухудшают.
- **D17 (Not doing per-request signing now):** Per-request Ed25519
  signing — правильный long-term путь (см. `work/stateless-auth-rearch/`),
  но breaking change для всех клиентов, месяцы работы. Stripe-precedent
  доказывает что refresh-токены достаточны для UX-проблемы.

## Тестирование

**Unit-тесты:** делаются всегда, не обсуждаются. Покрывают: token
generation, blake3-hashing с солью, storage CRUD, expiry check, rotation
chain, reuse-interval lookup, family-revoke walk.

**Интеграционные тесты:** делаем — нужно проверить полный round-trip
`authorization_code` → `refresh_token` через axum-router. Без них
можно случайно сломать wire-протокол, который клиенты смотрят.
Расширяем `TestServerBuilder` (`mcp/tests/_helpers/mod.rs:104-126`)
флагом `with_oauth_token(true)` чтобы он монтировал `/oauth/token` в
тот же tower-stack что и `/mcp` — паритет с прод-конфигурацией.
Per-test mini-router'ы НЕ используем (риск дрифта между тестовой и
прод router-конфигурацией).

**E2E тесты:** делаем минимальный — поднять сервер, прогнать
curl-based смок (`mcp/tests/oauth_refresh_e2e.rs`): авторизация →
ротация 1 → ротация 2 → replay старого → family-revoke. Плюс
**обязательный pre-ship empirical test** реального Claude Desktop по
R1 mitigation — НЕ в CI (Claude Desktop не headless), но **результат
этого теста — гейт на ship**. Без подтверждения что Claude Desktop
ротирует — фича помогает только Cursor/VS Code, и мы это фиксируем
осознанно.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. `POST /oauth/token grant_type=authorization_code` через TestServer (form-encoded) | cargo test integration | 200 с полями `access_token`, `refresh_token`, `expires_in=3600`, `token_type=Bearer` (AC1) |
| 2. То же что 1, но `Content-Type: application/json` | cargo test integration | Тот же 200 с теми же полями — паритет (AC10) |
| 3. С новым access-токеном вызвать `tools/call mnemonic_whoami` | cargo test integration | 200 с pubkey, `sub` от первого grant'а (AC8) |
| 4. Подождать 1ч (или симулировать exp в прошлом) → access протух | cargo test integration | 401 `-32001` на whoami, как сегодня (AC8) |
| 5. `POST /oauth/token grant_type=refresh_token refresh_token=<saved>` | cargo test integration | 200 с **новыми** access и refresh; новый `expires_at > старый_expires_at` (AC2, AC6) |
| 6. **Сразу** (внутри 30s) повторить шаг 5 с тем же старым `rt_X` | cargo test integration | 200 с **той же** descendant pair что в шаге 5 (reuse-interval, AC12) |
| 7. Параллельно (tokio::join!) выполнить 2 одновременных refresh-grant'а с одним `rt_X` | cargo test integration | Оба возвращают идентичные descendant tokens; семья НЕ revoked (AC12) |
| 8. Подождать 31s → повторить шаг 5 со старым `rt_X` | cargo test integration | 400 `invalid_grant` + family revoked (AC3, AC4) |
| 9. После шага 8 — попробовать использовать НОВЫЙ refresh из шага 5 | cargo test integration | 400 `invalid_grant` (вся семья revoked, AC4) |
| 10. `GET /.well-known/oauth-authorization-server` | cargo test integration | JSON содержит `"grant_types_supported": [..., "refresh_token"]` (AC7) |
| 11. Симулировать refresh с `expires_at < now` (1y+ протух) | cargo test integration | 400 `invalid_grant`; семья НЕ revoked (истечение ≠ атака, AC5) |
| 12. `POST /oauth/token grant_type=refresh_token` без поля `refresh_token` | cargo test integration | 400 `invalid_request` (AC13) |
| 13. `POST /oauth/token grant_type=authorization_code` без новой логики (просто игнорируем `refresh_token` в ответе) | cargo test integration | Работает как сегодня — wire-format back-compat (AC11) |

### Пользователь проверяет

- **Pre-ship Option A (рекомендую первым):** перехватить HTTPS-трафик
  на машине где работает Claude Desktop, подключённый к
  `https://mcp.stripe.com` (или к любому хорошо известному OAuth 2.1
  MCP-серверу с refresh-tokens). mitmproxy / Charles Proxy / Wireshark.
  Смотрим: делает ли Claude Desktop `POST /oauth/token grant_type=
  refresh_token` после истечения access? Если да — наш фикс
  сработает по тому же пути.
- **Pre-ship Option B:** dev-деплой `mcp.dev.mnemonik.xyz` с
  `JWT_TTL_SECS=60`. Подключить Claude Desktop, авторизоваться, ждать
  2 минуты. Смотрим: продолжает ли работать без OAuth-страницы?
  Параллельно — Cursor с тем же сервером (известно что поддерживает,
  служит control'ом).
- **После ship:** подключить Claude Desktop к prod
  `mcp.mnemonik.xyz` v0.2.5 → авторизоваться → оставить открытым 2+
  часа → сохранить заметку через `mnemonic_sign_memory`. Без
  OAuth-страницы — успех.
