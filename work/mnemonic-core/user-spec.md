---
created: 2026-04-20
status: draft
type: feature
size: L
---

# User Spec: mnemonic-core (Итерация 1 — Native extraction)

## Что делаем

Извлекаем всю доменную логику из монолитного MCP-сервера (`mnemonic-protocol/mcp/`) в отдельный Rust lib crate `mnemonic-core` с native-компиляцией. MCP-сервер становится тонкой обёрткой, зависящей от core как workspace member. Миграция поэтапная — `cargo test` и `cargo clippy` остаются зелёными после каждого шага. WASM-таргет — отдельная итерация 2.

## Зачем

Сейчас вся логика — embedding, сжатие, identity, хранилище, attestation, lineage — намертво вшита в MCP-бинарник. Это блокирует переиспользование: любой Rust-проект не может добавить только логику core без HTTP-сервера. Разделение является обязательным шагом перед публикацией `mnemonic-core` на crates.io и перед итерацией 2 (WASM для webapp).

## Как должно работать

### Сценарий 1 — разработчик на Rust

Разработчик добавляет `mnemonic-core` в свой `Cargo.toml`. Вызывает `identity::load_or_create_keypair(path)` — получает Ed25519 keypair. Инициализирует `FastEmbedder` (с feature `local-embed`) или `OpenAIEmbedder` (с `OPENAI_API_KEY`), embed текст, применяет `EmbeddingCompressor::compress()`. Если ни fastembed, ни OpenAI недоступны — `embed()` возвращает `Err("no embedder configured")`.

### Сценарий 2 — MCP-сервер работает как раньше

MCP запускается как раньше. Под капотом `mnemonic-mcp` зависит от `mnemonic-core` как workspace member. Все 5 MCP-инструментов (whoami, sign_memory, verify, prove_identity, recall) работают идентично текущему поведению. SQLite-хранилище, fastembed, Arweave, Solana — функционируют без изменений во внешнем поведении. Существующий `~/.mnemonic/attestations.db` продолжает работать без миграций.

## Критерии приёмки

- [ ] `cargo test -p mnemonic-core` — все тесты зелёные, включая 8 тестов lineage и httpmock-тесты для arweave/solana
- [ ] `cargo clippy -p mnemonic-core -- -D warnings` — ноль предупреждений
- [ ] `cargo build -p mnemonic-mcp` — компилируется с `mnemonic-core` как workspace dep
- [ ] MCP в local mode: JSON-RPC `tools/list` возвращает 5 инструментов; `sign_memory` сохраняет в SQLite; `recall` возвращает тот же контент — round-trip работает
- [ ] `turboquant-plus-rs = "0.1.0"` в `core/Cargo.toml` — crates.io, не git; импорты в `compress.rs` обновлены под новый namespace
- [ ] `lineage.rs`: ошибки БД propagate через `?`; `chain_valid: Option<bool>`; `Direction` — enum
- [ ] `HashEmbedder` отсутствует в `core/` — `grep -r "HashEmbedder" core/src/` пустой результат
- [ ] Payment-методы отсутствуют в `core/` — `grep -r "create_api_key\|deduct_balance\|credit_deposit\|mark_x402_nonce\|record_attestation_cost\|get_pnl_stats\|get_owner_pubkey\|verify_usdc_transfer" core/src/` пустой результат
- [ ] Benchmarks (`decompress`, `cbor_codec`) и proptest перенесены в `core/benches/` и `core/tests/`
- [ ] `architecture.md` содержит `codec/` и `lineage/` в описании `core/src/` — `grep -E "codec/|lineage/" .claude/skills/project-knowledge/references/architecture.md` выдаёт обе строки

## Ограничения

- **Поэтапность**: codec → identity → embed → compress → db/storage → arweave/solana → lineage. После каждого шага `cargo test` и `cargo clippy` зелёные.
- **Native-only**: WASM, `wasm/mod.rs`, `web_sys`, localStorage — вне scope.
- **Без изменений схемы БД**: существующий `~/.mnemonic/attestations.db` продолжает работать без миграций.
- **MCP API**: JSON-RPC API 5 инструментов сохраняется без изменений.
- **Публикация** на crates.io — вне scope. Отдельная задача.
- **`pricing.rs`**: остаётся в `mcp/` без изменений.

## Риски

- **Риск 1: turboquant import namespace.** `compress.rs` импортирует `turboquant::`, после смены на `turboquant-plus-rs` namespace становится `turboquant_plus_rs::`. Митигация: первый шаг миграции — только смена dep + обновление импортов + `cargo test`. Остальные шаги — после.
- **Риск 2: Нулевое покрытие arweave.rs и solana.rs.** Митигация: httpmock-тесты добавляются при переносе этих модулей — мокируем Irys и Solana RPC эндпоинты.
- **Риск 3: fastembed модель не скачана.** При первом запуске `FastEmbedder::try_new()` скачивает ~22MB модель. Митигация: в CI устанавливать fastembed cache; в тестах использовать `HashEmbedder`-эквивалент или mock. На самом деле HashEmbedder удалён — тесты embed должны или мокировать, или использовать `EMBED_PROVIDER=openai` в CI с заглушкой.

## Технические решения

- **Storage трейты**: `AttestationStore` и `LineageStore` — трейты в core; SQLite-имплементации в `core/src/storage/`; payment-методы остаются в `mcp/`.
- **`lineage.rs`** исправляется при переносе: Direction enum, chain_valid: Option<bool>, ошибки propagate.
- **`HashEmbedder` удалён**: не позволяет декомпрессию embeddings.
- **Workspace**: `mnemonic-refactored/Cargo.toml` — workspace `[core, mcp]`, resolver = "2".

## Тестирование

**Unit-тесты:** после каждого перенесённого модуля. Новые httpmock-тесты для arweave и solana.

**Интеграционные тесты:** MCP в local mode с новым core dep — все 5 инструментов через JSON-RPC.

**E2E тесты:** не делаем — нет задеплоенного окружения.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|-------------------|
| 1. Смена dep (первый шаг) | `bash: cargo test -p mnemonic-core` | Все тесты зелёные после обновления импортов |
| 2. После каждого модуля | `bash: cargo test -p mnemonic-core && cargo clippy -p mnemonic-core -- -D warnings` | Зелёный, ноль warnings |
| 3. MCP компилируется | `bash: cargo build -p mnemonic-mcp` | Успешно |
| 4. MCP round-trip | `bash: JSON-RPC calls via stdio в local mode` | sign_memory сохраняет, recall возвращает тот же контент |
| 5. Payment методы не в core | `bash: grep -r "create_api_key\|deduct_balance\|credit_deposit" core/src/` | Пустой результат |
| 6. HashEmbedder удалён | `bash: grep -r "HashEmbedder" core/src/` | Пустой результат |
| 7. architecture.md обновлён | `bash: grep -E "codec/\|lineage/" .claude/skills/project-knowledge/references/architecture.md` | Обе строки найдены |

### Пользователь проверяет

- Запустить `cargo run -p mnemonic-mcp` в local mode, вызвать `mnemonic_whoami` через Cursor или Claude Desktop — убедиться что pubkey совпадает с тем что был до миграции (keypair файл не менялся).
- Открыть `~/.mnemonic/attestations.db` через sqlite3, убедиться что старые записи на месте и `sign_memory` добавляет новые строки.
