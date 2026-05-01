---
created: 2026-05-01
status: draft
type: feature
size: L
---

# User Spec: A2A Bridge — Mnemonic as the durable memory layer for Agent2Agent workflows

## Что делаем

Шипаем «мост» между Mnemonic Protocol и Agent2Agent (A2A) — открытым протоколом межагентного взаимодействия (Google, v1.0.0-rc, JSON-RPC + SSE по HTTP). Bridge превращает каждое A2A-взаимодействие — `Task`, `Message`, `Artifact` — в подписанный, проверяемый, durable Mnemonic-attestation, индексированный по A2A `contextId`.

Конкретно поставляем:

1. **Три новые CBOR-схемы** в `mnemonic-core` (`A2A_TASK_V1`, `A2A_MESSAGE_V1`, `A2A_ARTIFACT_V1`) — детерминированные, COSE_Sign1, blake3-хешированные, пристёгнутые к существующему lineage-DAG через `prev_id`.
2. **Адаптер-крейт `mnemonic-a2a`** — pure-Rust функции `attest_message`, `attest_task`, `attest_artifact`, `recall_by_context` поверх `mnemonic_core`.
3. **Identity binding** — расширение AgentCard `x-mnemonic` (через A2A `Extension`-механизм), публикующее Ed25519 pubkey агента, чтобы потребитель A2A-карточки сразу мог проверять Mnemonic-подписи.
4. **Два новых MCP-инструмента** (`mnemonic_attest_a2a`, `mnemonic_recall_a2a`) на `mcp.mnemonik.xyz` — чтобы A2A-агент, уже подключённый к Mnemonic через MCP, мог фиксировать события в одну строчку.
5. **Reference-сайдкар `bridge-a2a/`** — минимальный axum-middleware который сидит перед A2A-сервером, перехватывает `SendMessage` / `GetTask completed` / artifact emission и автоматически производит attestation. Sidecar — это «опциональный путь, без переписывания агента»; библиотечный путь (`use mnemonic_a2a::attest_task`) — для тех, кто хочет интегрироваться напрямую.
6. **SDK-хелперы** в `@mnemonik-xyz/sdk`: `attestA2ATask`, `attestA2AArtifact`, `recallA2AContext` — чтобы TS/JS-агент получил тот же контракт, что и Rust-сторона.
7. **Conformance-фикстуры** — golden vectors `{a2a_task_json, canonical_cbor_hex, cose_envelope_hex}`, публикуемые отдельным npm-артефактом, чтобы любая сторонняя реализация могла доказать byte-for-byte parity.
8. **Threat-model** — новый документ `references/threat-model.md`, покрывающий A2A-границу: канонические mismatches, replay, forking, identity-substitution.

Schema-policy: фиксируем `*_V1` против A2A v1.0.0 GA. До GA — `experimental` в `decisions.md`.

## Зачем

A2A — это «движение», а не «память»: спецификация явно говорит, что агенты сотрудничают «without needing access to each other's internal state, memory, or tools». Tasks живут до `completed/failed`, после чего их сохранение — частное дело конкретного A2A-сервера. Никакого cross-session, cross-vendor, signed audit trail протокол не предлагает.

Whitepaper Mnemonic уже занимает эту нишу позиционно (`docs/WHITEPAPER.md`: «A2A makes agents interoperable in motion; Mnemonic makes them coherent over time»). Но в коде моста нет: каждый интегратор вынужден руками маппить `Task`/`Message`/`Artifact` на `MEMORY_V1` и тащить эту склейку через ребрейки A2A.

Что закрывает A2A bridge:

- **Шесть из одиннадцати use-case'ов** в `docs/usecases/` явно построены на A2A: `task-memory-ledger`, `shared-memory-layer`, `shared-project-memory-namespace`, `artifact-attestation-service`, `provenance-attestation-layer`, `reliability-oracle-for-orchestration`. Сейчас все они — «впишите glue-код сами». Bridge превращает их в «подключите sidecar / импортируйте крейт».
- **Дифференциация против letta / zep / mem0 / cognee** — никто из них не подписывает память и не имеет A2A-binding'а. Двойной moat: «мы attest» × «мы нативно говорим на multi-agent протоколе». Ретроспективно повторить трудно: схемы должны быть детерминированными и version-stable.
- **Reputation / reliability oracle становится исполнимым** — `contextId` как индексируемый ключ позволяет считать «agent X выполнил 2400 задач в этом контексте без разрывов lineage». Это переводит use-case `reliability-oracle-for-orchestration` и `trust-reputation-layer` из speculative в executable.
- **Композируется с AgentCard JWS** — A2A v0.3+ уже подписывает AgentCard через JWS (RFC 8785). Mnemonic-attestation — это естественный следующий слой: card доказывает «кто», Mnemonic-envelope — «что сделал».

## Как должно работать

### Сценарий 1 — Drop-in sidecar

DevOps команды разворачивает `bridge-a2a` как контейнер перед существующим A2A-сервером. На каждый `SendMessage` / `GetTask` (status=completed) / artifact-emission sidecar:

1. Канонизирует A2A JSON через RFC 8785 (как делает A2A AgentCard signing).
2. Оборачивает канонические байты в Mnemonic-CBOR-envelope с соответствующей `*_V1` схемой.
3. Подписывает через `mnemonic_core::codec::sign` Ed25519-ключом агента.
4. Кладёт в Mnemonic SQLite + (optional, full-mode) Arweave + Solana memo.
5. Возвращает ответ A2A с extension-полем `x-mnemonic: { attestation_id }`.

### Сценарий 2 — Library integration

Агент-разработчик импортирует `mnemonic_a2a` крейт (Rust) или `@mnemonik-xyz/sdk` (TS) и явно вызывает `attest_task(...)` после завершения задачи. Применимо когда нет separate A2A-сервера или хочется полный контроль над тем, что попадает в memory.

### Сценарий 3 — Cross-agent recall

Любой A2A-агент с access к Mnemonic-MCP вызывает `mnemonic_recall_a2a(context_id, query?)` и получает упорядоченный список всех attested сообщений / задач / артефактов в этом A2A-контексте, через любые vendor'ы и сессии. Это то, что A2A сама по себе не даёт.

### Сценарий 4 — Independent verification

Третья сторона получает A2A `Task` JSON + Mnemonic `attestation_id`. Через published conformance fixtures и любую реализацию `mnemonic-core` (включая Wasm-сборку) она перепроверяет: канонический CBOR совпадает, blake3 совпадает, COSE_Sign1 валиден против Ed25519-pubkey, опубликованного в `x-mnemonic` extension AgentCard'а. Никакого доверия к `mcp.mnemonik.xyz` не нужно.

## Out of scope (Phase 1 of the bridge)

- Streaming SSE chunk-level attestation — Phase 1 фиксирует только финальное состояние Task. Per-chunk attestation — backlog.
- Полная chain-pluggability anchor'а — наследуем текущий Solana lock из `core/`. После того как `mnemonic-cli` Phase 3 сделает chain-pluggable anchor, A2A bridge получит это бесплатно.
- Schema-эволюция выше V1 — V1 фиксируется против A2A v1.0.0 GA. Любые breaking changes — новый `*_V2`, не мутируем V1 (та же дисциплина что у `MEMORY_V1`).
- Non-A2A protocol integrations (MCP-to-MCP, ACP, AGNTCY) — см. `backlog.md`.
