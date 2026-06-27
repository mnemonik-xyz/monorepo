---
created: 2026-06-27
status: draft
type: feature
size: L
---

# User Spec: Verifiable Trajectories — доказуемые последовательности шагов агента

## Что делаем

Добавляем в Mnemonic слой **verifiable trajectories** — возможность доказать, что
последовательность шагов, выполненных агентом, была *хорошей*. «Хорошая» здесь
раскладывается на два независимых утверждения, и мы поставляем оба:

1. **Целостность последовательности** (chain integrity) — шаги шли именно в этом
   порядке и ни один не был подменён/вставлен/удалён. Это hash-linked цепочка
   подписанных шагов: каждый `step` несёт `prev_hash` = `content_hash`
   предыдущего шага, `seq` (порядковый номер) и `trajectory_id`.
2. **Привязка вердикта** (verdict binding) — каждый шаг несёт ссылку на вердикт
   о его корректности/качестве, **подписанный независимой identity-судьёй**
   (PRM, LLM-judge, детерминированная проверка, или внешний zkML/TEE/OCP-пруф).
   Mnemonic подписывает то, *что вердикт существует и пристёгнут к шагу* — не то,
   что математика модели верна.

Конкретно поставляем:

1. **Три новые CBOR-схемы** в `mnemonic-core`: `STEP_V1`, `TRAJECTORY_V1`,
   `VERDICT_V1` — детерминированные, COSE_Sign1, blake3, пристёгнутые к
   существующему lineage-DAG.
2. **Chain-верификатор** — материализация поля `chain_valid` (сегодня всегда
   `None` в `core/src/lineage/mod.rs`): обход упорядоченной цепочки + проверка
   `prev_hash`-связности и подписи каждого узла.
3. **Per-trajectory Merkle batch root** — расширение `core/src/merkle.rs`:
   один корень на траекторию по упорядоченным хешам шагов + inclusion-proof на
   любой шаг (логарифмическое доказательство существования шага во времени).
4. **Storage — децентрализованно, без БД на стороне MCP** (решение владельца
   2026-06-27): «keep everything» через Arweave ANS-104 bundles. Единица
   записи — bundle, а не шаг (та же Merkle-batch идея на слой ниже), поэтому
   «хранить всё» становится доступным по цене. MCP-сервер — stateless
   verifier/relay; источник истины = permaweb (контент) + anchor-цепочка
   (timestamps корней) + keychain пользователя (identity). BYO-wallet: байты
   принадлежат пользователю. `SqliteStore` — только опциональный локальный кэш.
   Recall — Arweave GraphQL по тегам + cosine на клиенте (V1).
5. **Три MCP-инструмента**: `mnemonic_attest_step` (зафиксировать шаг сейчас),
   `mnemonic_attest_verdict` (пристегнуть вердикт, в т.ч. асинхронно),
   `mnemonic_verify_trajectory` (вернуть `chain_valid`, batch root, inclusion
   proofs, покрытие вердиктами).
6. **Decoupled prove** (паттерн ERC-8301 `onAgentStep` / `onAgentProve`) —
   шаг коммитится немедленно, вердикт может прийти позже через уже существующий
   механизм `correlation_id` / `check_pending`. Инвариант: ни один `is_final`
   или high-value action не разрешён, пока у всех предшествующих шагов нет
   вердикта на записи.
7. **Conformance-фикстуры + threat-model** — golden vectors для 3-шаговой
   траектории и документ угроз (reordering, verdict-forgery, omission,
   judge-substitution, batch-root mismatch).

Schema-policy: `*_V1` экспериментально (gated за cargo-feature
`trajectory-experimental`), пока не зафиксируем GA в `decisions.md`.

## Зачем

Сегодня Mnemonic даёт **Proof of Existence** — подпись доказывает, что некий агент
произвёл некий артефакт. Но lineage-DAG хранит только `parent_id` и
*нарративные* роли («context», «state», «trigger») — это **утверждение** о связях,
а не tamper-evident цепочка. Поля `seq` и `prev_hash` нет; `chain_valid` никогда
не вычисляется. Агент может подписать галлюцинацию так же легко, как факт.

По мере выхода агентов в high-consequence домены (финансы, право, медицина)
рынку нужен переход от «кто-то это произвёл» к «последовательность шагов была
проверяемо корректной». Возникающие стандарты (ERC-8274 `IProofVerifier` /
OCP-примитив `recompute → compare → confirm inclusion`, ERC-8263 `anchor`,
ERC-8301 decoupled step/prove) и живые игроки (invinoveritas / Baby Blue Viper:
verdict, закоммиченный *до* исхода, Schnorr-proof, OpenTimestamps) уже строят
этот слой.

**Позиционирование (locked 2026-05-01):** *Mnemonic — verifiable memory for
trustless agents.* Verifiable trajectories — это прямое продолжение: мы занимаем
**слой commitment + lineage** (третья нога OCP — «confirm inclusion», которую
`merkle.rs` уже делает), а корректность вычисления (zkML/opML/TEE) — **связываем,
но не производим**. Это композиция, не конкуренция: судья (PRM / invinoveritas /
TEE) выдаёт вердикт, Mnemonic даёт ему порядок, подпись, durability и recall.

Что закрывает фича:

- Переводит шесть use-case'ов вокруг reliability/provenance из «впишите glue сами»
  в исполнимые: траектория с `chain_valid: true` и 100%-покрытием вердиктами —
  это и есть «agent X выполнил задачу без разрывов lineage и с пройденными
  проверками на каждом шаге».
- Даёт чистую точку интеграции под ERC-8274/8004 backlog (`work/a2a-bridge/`):
  batch root анкорится как `proofHash`, вердикты ложатся в Validation/Reputation
  Registry.
- Не нарушает архитектуру: `core/` остаётся native-only, граф зависимостей
  однонаправленный, никакого zk-прувера в ядре.

## Как должно работать (happy path)

1. Агент на каждом шаге зовёт `mnemonic_attest_step` → шаг подписан, получает
   `content_hash`, ссылается на `prev_hash` предыдущего. Дешёво, `local`-mode по
   умолчанию.
2. Судья (PRM / внешний сервис) оценивает шаг → `mnemonic_attest_verdict`
   пристёгивает подписанный судьёй `VERDICT_V1` к шагу (синхронно или позже по
   `correlation_id`).
3. По завершении задачи `mnemonic_verify_trajectory` строит batch root,
   проверяет `chain_valid`, покрытие вердиктами и возвращает inclusion proofs.
   Только `participate`-mode анкорит batch root на Arweave + Solana.
4. Любой третий участник проверяет: цепочку (prev_hash + подписи), вердикты
   (подпись судьи), и существование любого шага под анкоренным корнем — всё
   офлайн, без доступа к весам модели.

## Вне V1 (явно)

- Производство zkML/opML/TEE-пруфов внутри Mnemonic. Только связывание по хешу.
- Генерация самих PRM-оценок. Mnemonic не судья — он подписывает вердикт судьи.
- On-chain верификатор-контракт (ERC-8274 handler). Наследуется из chain-pluggable
  anchor / ERC-8004 backlog.
- Семантический recall по шагам траектории (отдельный backlog).
