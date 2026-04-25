---
created: 2026-04-25
status: draft
type: feature
size: L
---

# User Spec: mnemonic-webapp (MVP -- Protocol Chatbot)

## Что делаем

React webapp с бэкендом на MCP-сервере. Протокольный чатбот на базе RAG + Ollama (Qwen2.5-3B), который отвечает на вопросы о Mnemonic Protocol, используя предзагруженные аттестованные знания (whitepaper + docs). Пользователь может скачать knowledge artifact (Markdown + JSON sidecar) и использовать как стартовый контекст в любом другом AI-инструменте (ChatGPT, Claude, Cursor, Gemini, локальные LLM).

## Зачем

Сейчас узнать о Mnemonic Protocol можно только прочитав whitepaper или код. Нет интерактивного способа познакомиться с протоколом. Webapp решает две задачи:

1. **Демо-витрина протокола.** Non-technical users задают вопросы в чате и получают ответы, основанные на аттестованных знаниях -- живая демонстрация RAG + semantic recall.

2. **Cross-provider портативность.** Пользователь скачивает протокольные знания как артефакт с криптографическими метаданными и загружает в свой AI-инструмент -- начинает работу с уже подготовленным контекстом о протоколе. Это демонстрация ключевой ценности Mnemonic: knowledge, который переживает смену провайдера.

## Как должно работать

### Сценарий 1 -- Новый пользователь знакомится с протоколом

Пользователь открывает сайт. Видит landing page с кратким описанием Mnemonic Protocol и кнопкой "Start chat". Нажимает -- попадает в чат-интерфейс. Пишет "What is Mnemonic Protocol?" -- получает ответ, основанный на whitepaper (RAG recall + Ollama). Продолжает диалог, задаёт уточняющие вопросы. Чатбот отвечает только на основе предзагруженных знаний о протоколе.

### Сценарий 2 -- Скачивание knowledge artifact

Пользователь видит кнопку "Download protocol knowledge" (всегда видна в UI, не зависит от чата). Нажимает -- скачивает `.zip` с двумя файлами:

- `mnemonic-protocol-knowledge.md` -- Markdown с YAML frontmatter (content_hash, signer_pubkey, timestamp, arweave_tx) и текстовым содержимым всех протокольных знаний.
- `mnemonic-protocol-knowledge.json` -- JSON с структурированными метаданными, подписями, и опционально quantized embeddings в base64.

Пользователь загружает `.md` файл в ChatGPT/Claude/Gemini как context file или вставляет текст в system prompt локальной LLM -- и продолжает разговор о протоколе с полным контекстом.

### Сценарий 3 -- Ограничения сессии

При достижении 50 сообщений в сессии пользователь видит "Session limit reached. Refresh to start a new session." При rate limit (10 req/min) -- "Too many requests. Please wait a moment." При ошибке сервера -- автоматический retry (2-3 попытки), затем "Service temporarily unavailable. Try again later."

## Критерии приёмки

- [ ] Landing page загружается, содержит описание протокола и кнопку "Start chat"
- [ ] Чат-интерфейс отображается в тёмной теме (background `#0A0F1E`, accent `#00D4B4`)
- [ ] Пользователь вводит вопрос о протоколе -- получает релевантный ответ в течение 15 секунд
- [ ] Ответы чатбота основаны на протокольных знаниях (не галлюцинации) -- проверка: вопрос "What are the 5 MCP tools?" возвращает все пять: whoami, sign_memory, verify, prove_identity, recall
- [ ] Кнопка "Download protocol knowledge" скачивает `.zip` файл
- [ ] `.zip` содержит `.md` файл с валидным YAML frontmatter (поля: content_hash, signer_pubkey, timestamp)
- [ ] `.zip` содержит `.json` файл с теми же метаданными в структурированном формате
- [ ] Предзагрузка знаний: при первом запуске сервера whitepaper разбивается на секции и сохраняется через `sign_memory`; при повторном запуске -- пропускается (проверка `attestation_count > 0`)
- [ ] Rate limit: 11-й запрос в минуту с одного IP возвращает HTTP 429 с сообщением
- [ ] Session limit: после 50 сообщений UI показывает уведомление и блокирует ввод
- [ ] При недоступности Ollama -- после 2-3 retry отображается "Service temporarily unavailable"
- [ ] `POST /chat` endpoint на MCP-сервере принимает `{"message": "...", "session_id": "..."}` и возвращает `{"response": "..."}`
- [ ] `docker compose up` на сервере с 6 vCPU / 12GB RAM поднимает nginx + MCP + Ollama
- [ ] Playwright E2E: открыть сайт → нажать "Start chat" → отправить вопрос → получить ответ → скачать artifact

## Ограничения

- **Без подписания пользователем.** Keypair, client-side signing, COSE_Sign1 в браузере -- deferred. Артефакты подписаны серверным keypair.
- **Без аутентификации.** Open access, нет логина/регистрации.
- **Без истории сессий.** Обновление страницы = новая сессия. Нет persist между визитами.
- **Один shared knowledge store.** Все пользователи видят одни и те же предзагруженные знания.
- **Только протокольные знания.** Чатбот не отвечает на вопросы вне контекста Mnemonic Protocol.
- **Research agent** -- отдельная фича, вне scope.
- **Шифрование keypair** в localStorage -- вне scope (нет keypair в MVP).

## Риски

- **Риск 1: Ollama cold start.** Первый запрос после старта контейнера может занять 30-60 секунд (загрузка модели в RAM). Митигация: health check + warm-up запрос при старте контейнера.
- **Риск 2: Качество ответов Qwen2.5-3B.** 3B модель может давать неточные ответы на сложные вопросы о протоколе. Митигация: system prompt жёстко ограничивает -- отвечай только на основе context, не додумывай; RAG подставляет релевантные секции.
- **Риск 3: Размер whitepaper для chunking.** Whitepaper ~20K токенов. При разбивке по секциям некоторые секции могут быть слишком большими для context window 3B модели (~4K). Митигация: разбить крупные секции на подсекции (### уровень).

## Технические решения

- **Frontend:** React + Vite + Tailwind CSS. Тёмная тема per UX guidelines. Deployment: static files served by nginx на том же сервере.
- **Backend:** Существующий MCP-сервер + новый `POST /chat` endpoint. Endpoint flow: recall top-3 chunks → build prompt (system instruction + context + user message) → POST to Ollama `/api/generate` → stream/return response.
- **RAG seeding:** Startup скрипт разбивает `docs/WHITEPAPER.md` по `##` секциям. Каждая секция -- отдельный вызов `sign_memory` с тегом `["protocol-knowledge", "whitepaper"]`. Пропускается если `attestation_count > 0`.
- **LLM:** Ollama + `qwen2.5:3b` model. System prompt: "You are an expert on the Mnemonic Protocol. Answer questions ONLY based on the provided context. If the context does not contain the answer, say so."
- **Artifact download:** `/download-knowledge` endpoint собирает все аттестации с тегом `protocol-knowledge`, генерирует Markdown (YAML frontmatter + content) и JSON sidecar, возвращает `.zip`.
- **Deploy:** Docker Compose с тремя сервисами: `nginx` (static React + reverse proxy), `mcp` (MCP-сервер), `ollama` (Qwen2.5-3B). Один сервер justhost.asia, 6+ vCPU, 12GB+ RAM.

## Тестирование

**Unit-тесты:** `/chat` endpoint (mock Ollama, mock recall), artifact generation (проверка структуры .md и .json), rate limiter, session counter.

**Интеграционные тесты:** Полный RAG pipeline -- реальный recall с предзагруженными знаниями → формирование prompt → вызов Ollama → проверка что ответ содержит релевантную информацию.

**E2E тесты (Playwright):** 
- Golden path: открыть → chat → получить ответ → скачать artifact
- Rate limit: отправить 11 запросов за минуту → увидеть ошибку
- Session limit: отправить 50 сообщений → увидеть уведомление
- Error state: остановить Ollama → отправить запрос → увидеть error message

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. Docker Compose | `bash: docker compose up -d && docker compose ps` | Все 3 сервиса running |
| 2. Health check | `bash: curl http://localhost:3000/health` | `{"status":"ok"}` |
| 3. Chat endpoint | `bash: curl -X POST http://localhost:3000/chat -d '{"message":"What is Mnemonic?","session_id":"test"}'` | JSON response с релевантным ответом |
| 4. Knowledge seeding | `bash: curl -X POST http://localhost:3000/mcp -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemonic_whoami"}}'` | `attestation_count > 0` |
| 5. Artifact download | `bash: curl -o knowledge.zip http://localhost:3000/download-knowledge && unzip -l knowledge.zip` | Содержит .md и .json файлы |
| 6. Rate limit | `bash: for i in $(seq 12); do curl -s -o /dev/null -w "%{http_code}" http://localhost:3000/chat ...; done` | 11-й запрос = 429 |
| 7. Playwright | `bash: npx playwright test` | Все E2E тесты зелёные |

### Пользователь проверяет

- Открыть сайт в браузере, убедиться что landing page отображается корректно.
- Нажать "Start chat", задать вопрос "What are the 5 MCP tools?" -- убедиться что ответ содержит все 5 инструментов.
- Нажать "Download protocol knowledge" -- открыть .md файл, убедиться что содержит YAML frontmatter и текст о протоколе.
- Загрузить .md файл в ChatGPT как context file, задать вопрос о протоколе -- убедиться что ChatGPT отвечает на основе загруженного контекста.
