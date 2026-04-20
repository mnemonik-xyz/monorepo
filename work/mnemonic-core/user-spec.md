---
created: 2026-04-20
status: draft
type: feature
size: L
---

# User Spec: mnemonic-core

## Что делаем

Извлекаем всю доменную логику из монолитного MCP-сервера (`mnemonic-protocol/mcp/`) в отдельный Rust lib crate `mnemonic-core` с двойной целевой компиляцией: native (для MCP-сервера) и `wasm32-unknown-unknown` (для браузерного webapp). MCP-сервер становится тонкой обёрткой, зависящей от core как workspace member. Миграция поэтапная — `cargo test` остаётся зелёным после каждого шага.

## Зачем

Сейчас вся логика — embedding, сжатие, identity, хранилище, attestation, lineage — намертво вшита в MCP-бинарник. Это блокирует три ценности: (1) **webapp без бэкенда** — WASM в браузере позволяет пользователю работать без запущенного сервера; (2) **переиспользование** — любой Rust-проект может добавить `mnemonic-core` как зависимость без MCP; (3) **публикация** — `@mnemonic/core` на npm и `mnemonic-core` на crates.io как самостоятельные артефакты протокола (в следующей итерации).

## Как должно работать

### Сценарий 1 — разработчик на Rust

Разработчик добавляет `mnemonic-core = "0.1"` в свой `Cargo.toml`. Импортирует `mnemonic_core::identity`, вызывает `load_or_create_keypair(path)`, получает Ed25519 keypair. Вызывает `mnemonic_core::embed::FastEmbedder::try_new()`, embed текст, вызывает `mnemonic_core::compress::EmbeddingCompressor::compress()` — всё без MCP-сервера.

### Сценарий 2 — webapp в браузере

Webapp загружает WASM-билд core. При первом запуске: `whoami()` — если keypair нет в localStorage, генерируется новый, сохраняется, возвращается pubkey. При повторном визите keypair подгружается из localStorage. Пользователь вводит текст → `sign_memory(content)` → attestation сохраняется в localStorage. `recall(query, api_key)` → возвращает топ-k релевантных воспоминаний (требует OpenAI API key для embedding).

### Сценарий 3 — MCP-сервер

MCP запускается как раньше. Под капотом `mnemonic-mcp` теперь зависит от `mnemonic-core` как workspace member. Все 5 MCP-инструментов (whoami, sign_memory, verify, prove_identity, recall) работают идентично текущему поведению в local mode.

## Критерии приёмки

- [ ] `cargo test -p mnemonic-core` — все тесты зелёные, включая 8 тестов lineage и новые httpmock-тесты для arweave/solana
- [ ] `wasm-pack build --target web` в `core/` — компилируется без ошибок и предупреждений
- [ ] `wasm-pack test --headless --chrome` — `whoami()` возвращает валидный Ed25519 pubkey (base58)
- [ ] `cargo build -p mnemonic-mcp` — компилируется с `mnemonic-core` как workspace dep
- [ ] MCP в local mode: `tools/list` возвращает 5 инструментов, `mnemonic_whoami` отвечает корректно
- [ ] CI (GitHub Actions): PR блокируется при провале любого из вышеперечисленных шагов
- [ ] `turboquant-plus-rs = "0.1.0"` в `core/Cargo.toml` — зависимость через crates.io, не git
- [ ] `lineage.rs`: `traverse_lineage` возвращает `Err` при ошибке БД (не глотает); `chain_valid: Option<bool>`, `None` пока верификация не запущена; `Direction` — enum, не `&str`
- [ ] `HashEmbedder` удалён из кодовой базы
- [ ] Criterion benchmarks (`decompress`, `cbor_codec`) и proptest перенесены в `core/benches/` и `core/tests/`
- [ ] `architecture.md` обновлён: `core/src/` включает `codec/` и `lineage/`
- [ ] Payment-методы (`create_api_key`, `deduct_balance`, `get_pnl_stats` и др.) остаются в `mcp/`, не входят в core API

## Ограничения

- **Поэтапность**: каждый шаг миграции (codec → identity → embed → compress → db/storage → arweave/solana → lineage) заканчивается рабочим состоянием. Нельзя переносить сразу всё.
- **WASM**: `db` и `lineage` SQLite-реализации исключены из `wasm32` target (`#[cfg(not(target_arch = "wasm32"))]`). WASM-билд использует `web_sys::Storage` (localStorage) вместо SQLite.
- **Хранилище WASM**: localStorage лимит ~5-10MB. При переполнении — возвращать ошибку `"storage full"`. Формат хранения: JSON, ключ — attestation_id.
- **Embedder в WASM**: `fastembed` (ONNX) — native-only. В WASM доступен только OpenAI embedder (через `reqwest` wasm feature = browser fetch). Без OpenAI API key — `recall` возвращает ошибку `"no embedder configured"`.
- **Публикация** на crates.io и npm — **вне scope** этой задачи. Отдельная итерация.
- **MCP API**: JSON-RPC API 5 инструментов сохраняется. Внутренние изменения допустимы, внешний интерфейс — нет.
- **turboquant-plus-rs**: API v0.1.0 верифицирован совместимым (ndarray ^0.16 совпадает). Первый шаг миграции — смена dep с git на crates.io и прогон `cargo test`.

## Риски

- **Риск 1: turboquant-plus-rs API расходится с git-версией.** Митигация: первым шагом меняем только dep (git → crates.io), прогоняем `cargo test` — при несовместимости сразу видим до любых других изменений.
- **Риск 2: Отсутствие тестов для arweave.rs и solana.rs.** Митигация: добавляем httpmock-тесты при переносе этих модулей — мокируем HTTP-эндпоинты Irys и Solana RPC, покрываем happy path и network error.
- **Риск 3: Непредвиденная WASM-несовместимость в транзитивных зависимостях** (например `getrandom` без js-feature). Митигация: WASM build в CI на каждый PR — несовместимость обнаруживается немедленно.

## Технические решения

- **Storage trait abstraction**: `AttestationStore` и `LineageStore` — трейты в `core/`. SQLite-имплементации — native-only за `#[cfg(not(target_arch = "wasm32"))]`. `web_sys::Storage`-имплементации — в `core/src/wasm/storage.rs`. Payment-методы (`create_api_key`, `deduct_balance` и др.) остаются в `mcp/`, не переезжают в core.
- **HashEmbedder удалён**: не позволяет декомпрессию embedding, бесполезен как провайдер.
- **`wasm/mod.rs` экспортирует 4 функции**: `whoami() → String`, `sign_memory(content: &str) → JsValue`, `recall(query: &str, api_key: &str) → JsValue`, `verify(id: &str) → JsValue`. Все ошибки конвертируются в `JsValue` только на этой границе.
- **`codec/sign.rs` использует `solana-sdk`** для `Keypair`/`Signature` — WASM-совместимо для крипто-операций, рефакторинг на `ed25519-dalek` не нужен.
- **`lineage.rs` исправления**: `Direction` enum вместо `&str`, `chain_valid: Option<bool>`, ошибки БД propagate через `?`.
- **Workspace структура**: `mnemonic-refactored/Cargo.toml` — workspace `[core, mcp]`, resolver = "2".
- **`pricing.rs`** остаётся в `mcp/` без изменений (нет зависимостей от core-модулей).

## Тестирование

**Unit-тесты:** делаются всегда. Каждый модуль тестируется после переноса в core. Включают новые httpmock-тесты для `arweave.rs` и `solana.rs`.

**Интеграционные тесты:** делаем — запуск `mnemonic-mcp` в local mode с новым core dep, проверка всех 5 MCP-инструментов через JSON-RPC.

**E2E тесты:** не делаем — нет задеплоенного окружения, MCP работает локально. WASM smoke-тест (`wasm-pack test --headless --chrome`) покрывает браузерный путь.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. Смена dep на crates.io | `bash: cargo test -p mnemonic-core` | Все тесты зелёные |
| 2. После каждого перенесённого модуля | `bash: cargo test -p mnemonic-core` | Зелёный, без регрессий |
| 3. После финального переноса | `bash: wasm-pack build --target web` в `core/` | Компилируется без ошибок |
| 4. WASM smoke | `bash: wasm-pack test --headless --chrome` | `whoami()` возвращает base58 pubkey |
| 5. MCP интеграция | `bash: cargo build -p mnemonic-mcp` | Бинарник собирается |
| 6. MCP local mode | `bash: echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' \| cargo run -p mnemonic-mcp` | Возвращает 5 инструментов |
| 7. lineage bugs | `bash: cargo test -p mnemonic-core lineage` | Все 8 тестов + проверка Direction enum |

### Пользователь проверяет

- Открыть webapp в браузере → проверить что keypair сохранился в localStorage (DevTools → Application → Local Storage) после первого `whoami()` — при перезагрузке страницы тот же pubkey.
