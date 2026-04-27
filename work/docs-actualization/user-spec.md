---
created: 2026-04-26
status: approved
type: feature
size: M
---

# User Spec: docs-actualization

## Что делаем

Актуализируем протокольную документацию из staging-области `.claude/skills/project-knowledge/recovered/`. За одну фичу:

1. Восстанавливаем недостающие файлы из локального клона `/Users/syi/src/mnemonic-protocol` (ветка `origin/docs/usecases`, HEAD `7a68a973` — совпадает с pin в `recovered/README.md`). Pre-flight проверено. **Тремя группами (всего 11 файлов: 9 .md + 2 PDF):**
   - **competitive-landscape (3 файла):** `DRAG_ANALYSIS.md`, `WEB_RESEARCH_TRUSTLESS_RAG.md`, `DECENTRALIZED_RAG_LANDSCAPE.md` (последний явно подтверждён как «field landscape»). `MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` **исключён по решению владельца** — не восстанавливается, не архивируется. Подпапка `docs/historical/` не создаётся.
   - **research (3 .md + 2 PDF):**
     - `.md`: `TURBOQUANT_DEEP_ANALYSIS.md` (truncated upstream, restore as-is с recovery note), `apply-to-agent-memory-architecture.md`, `condensed-principles.md` (нужен как knowledge-DB ref).
     - PDF: `Agent Identity for Autonomous AI_ Protocols, Mnemonic Analysis, and the Path to a Minimal Primitive.pdf` (из root upstream) — Mnemonic-positioning analysis paper. `research/paper.pdf` (из upstream `research/`) — **foundational scientific paper**, мотивировавший проект; обязателен ref в `docs/WHITEPAPER.md` §References и в repo `README.md`.
   - **problems (3 файла, из `docs/<file>`):** `MEMORY_EVICTION.md` (system problem statement), `CONCURRENT_WRITERS.md` (multi-agent shared context), `ARWEAVE_PRICING_VALIDATION.md` (economic model). Кладутся в новую подпапку `recovered/problems/`.

   Восстанавливаются как binary/text через `git show`. Расширение `recovered/README.md` (новые строки таблицы для problems/, обоих PDF и пометка foundational у `paper.pdf`; запись «MCP_SERVER_BACKEND_FEATURES_COMPARISON.md — dropped per owner decision») — часть restoration commit.
2. Прогоняем sanity-grep всех recovered-документов (включая 3 новых) против текущего `core/` и `mcp/` на предмет устаревших утверждений (SHA3, mcp-server-rs, "pre-V1 prototype", HashEmbedder, "Python backend" и т.п.).
3. Промоутим evergreen-материал в публичный `docs/` (`usecases/`, `competitive-landscape/`, `research/`, `historical/`, новый **`problems/`**).
4. Применяем точечные правки в `.claude/skills/project-knowledge/references/` и расширяем `docs/WHITEPAPER.md` §9.
5. Сохраняем follow-up roadmap в `work/docs-actualization/decisions.md`. **CRITICAL_REVIEW.md** не восстанавливается — устарел; вместо этого в follow-ups добавляется bullet "redo critical review against current Rust impl".
6. Прочие upstream-файлы (`MVP_SPEC.md`, `DEMO_SPEC.md`, `MVP_VERIFICATION.md`, `v0/v1/v1.1` SCOPE docs, `mcp_server_rs/{API,SPEC}.md`, `report.md`, `PROJECT_STATE.md`, `diagrams/*.mmd`) явно отмечены как outdated и **не тащатся**.

Recovered-staging остаётся как audit trail с пометкой о промоушене.

## Зачем

Evergreen-знания протокола (A2A use-case роли, конкурентный ландшафт vs D-RAG/zkTAM, обоснование TurboQuant) написаны для прототипа `sivo4kin/mnemonic-protocol`. Часть была случайно уничтожена и восстановлена в staging-область, часть всё ещё отсутствует. Без промоушена в `docs/` и без обновления `project-knowledge`:

- внешние читатели (контрибьюторы, ресёрчеры) не видят полной positioning-картины;
- AI-агенты (project-knowledge, tech-spec-planning, code-writing) опираются на устаревший контекст и генерируют расходящиеся со shipped Rust-стейтом спеки;
- направления дальнейшей разработки (encryption, ZK proofs, shared namespaces, browser-WASM verification) теряются в мусоре.

Цель — синхронизировать публичные доки с текущим Rust-кодом и собрать chunk дальнейшей разработки в одном месте.

## Как должно работать

### Сценарий 1 — внешний разработчик

Разработчик открывает `docs/` на GitHub или mnemonik.xyz, читает `docs/usecases/` чтобы понять A2A-интеграцию, `docs/competitive-landscape/` для сравнения с D-RAG/zkTAM, `docs/research/` для TurboQuant-обоснования. Раздел §9 в `WHITEPAPER.md` даёт one-page обзор всех 10 use cases с deep-dive ссылками. Никаких упоминаний Python-бэкенда, SHA3 или "pre-V1 prototype" нет (за исключением `docs/historical/`).

### Сценарий 2 — AI-агент с project-knowledge

Агент при старте читает `.claude/skills/project-knowledge/references/{project,architecture,patterns}.md`. Видит обновлённый список use-case ролей в `project.md` и pointer на `docs/competitive-landscape/` в `architecture.md`. Генерируемые спеки ссылаются на shipped Rust state, а не на прототип.

### Сценарий 3 — будущий tech-spec автор

Автор открывает `work/docs-actualization/decisions.md`, читает раздел "Follow-up roadmap items". Видит развёрнутую sub-секцию про **Browser-WASM verification UI** (problem + proposed approach + dependencies + open questions + ссылки на recovered-доки) и bullet-list из 6 candidate-направлений (encryption, ZK proofs, shared namespaces, reliability oracle, compressed shadow-index recall, lifecycle policy) с пометкой "for further validation". Берёт WASM-пункт в работу через `/new-user-spec`.

## Критерии приёмки

**Восстановление**

- [ ] `recovered/competitive-landscape/{DRAG_ANALYSIS,WEB_RESEARCH_TRUSTLESS_RAG,DECENTRALIZED_RAG_LANDSCAPE}.md` присутствуют (MCP_SERVER_BACKEND_FEATURES_COMPARISON.md явно НЕ восстанавливается)
- [ ] `recovered/research/{TURBOQUANT_DEEP_ANALYSIS,apply-to-agent-memory-architecture,condensed-principles}.md` присутствуют
- [ ] `recovered/research/Agent Identity for Autonomous AI_ Protocols, Mnemonic Analysis, and the Path to a Minimal Primitive.pdf` присутствует
- [ ] `recovered/research/paper.pdf` присутствует (foundational scientific paper)
- [ ] `recovered/problems/{MEMORY_EVICTION,CONCURRENT_WRITERS,ARWEAVE_PRICING_VALIDATION}.md` присутствуют
- [ ] `recovered/README.md` обновлён: добавлены строки таблицы про `problems/` (3 файла), оба PDF в `research/`, с пометкой "foundational paper" у `paper.pdf`
- [ ] Restoration-коммиты ссылаются на `sivo4kin/mnemonic-protocol@docs/usecases` (commit hash `7a68a973` в commit message)

**Валидация**

- [ ] `work/docs-actualization/code-research.md` содержит per-hit таблицу sanity-grep'а (термины: `SHA3`, `mcp-server-rs`, `pre-V1`, `Pre-V1`, `HashEmbedder`, `Python backend`) с verdict-ом: `delete-section` | `drop-file` | `replace-token` (override). Pre-flight зафиксированные 6 hit'ов из 3 файлов (DRAG_ANALYSIS:37; WEB_RESEARCH:45,64,132; CONCURRENT_WRITERS:157,217) применены как 1-token replacements:
  - `SHA3-256` → `blake3` (везде, где относится к hash artifact'а)
  - `Pre-V1, prototype validated` → `active Rust MCP server`
  - `Pre-V1` → `v1.0 (active)`
  - `(SHA3 hash)` → удаление фразы в скобках
  - `SHA3-256(encrypted_blob)` → `blake3(canonical CBOR bytes)`
- [ ] Каждый `drop-file` verdict обоснован правилом ≥50% stale (из 9 .md ни один не пересекает порог per pre-flight)
- [ ] Каждый `replace-token` override залогирован в `code-research.md` с before/after
- [ ] `lychee --offline docs/` exits 0
- [ ] `grep -RIE 'SHA3|mcp-server-rs|pre-V1|Pre-V1|HashEmbedder|Python backend' docs/` находит ноль hits (поскольку `docs/historical/` не создаётся; `-I` skip binaries для PDF)

**Промоушен**

- [ ] `docs/usecases/` — 10 use-case .md + README
- [ ] `docs/competitive-landscape/` — `DRAG_ANALYSIS.md`, `WEB_RESEARCH_TRUSTLESS_RAG.md`, `DECENTRALIZED_RAG_LANDSCAPE.md` + README
- [ ] `docs/research/` — `TURBOQUANT_DEEP_ANALYSIS.md` (с recovery note), `apply-to-agent-memory-architecture.md`, `condensed-principles.md`, оба PDF (Agent-Identity и paper.pdf) + новый README
- [ ] `docs/problems/` — `MEMORY_EVICTION.md`, `CONCURRENT_WRITERS.md`, `ARWEAVE_PRICING_VALIDATION.md` + новый README (открытые системные проблемы + pricing-валидация, влияющие на roadmap)
- [ ] `docs/historical/` НЕ создаётся (MCP_SERVER_BACKEND_FEATURES_COMPARISON.md дропнут)

**Правки документации**

- [ ] `WHITEPAPER.md` §9 перечисляет все 10 use cases; каждая запись = 1-2 предложения + ссылка на `docs/usecases/<file>.md`
- [ ] `WHITEPAPER.md` §References содержит запись на foundational `docs/research/paper.pdf` (relative link)
- [ ] `README.md` (repo root) содержит секцию (или явную строку в Introduction) со ссылкой на `docs/research/paper.pdf` как foundational paper, мотивировавший проект
- [ ] `.claude/skills/project-knowledge/references/project.md` содержит секцию "Use Case Roles" со ссылками на `docs/usecases/`
- [ ] `.claude/skills/project-knowledge/references/architecture.md` содержит pointer-параграф на `docs/competitive-landscape/` и на `docs/research/condensed-principles.md` (краткое описание принципов TurboQuant — нужно как knowledge-DB ref)
- [ ] `patterns.md` без изменений (если sanity-grep ничего не показал)

**Follow-up roadmap**

- [ ] `work/docs-actualization/decisions.md` содержит секцию "Follow-up roadmap items"
- [ ] Sub-секция "Browser-WASM verification UI" имеет: Problem, Proposed Approach, Dependencies, Open Questions, Source-doc refs
- [ ] Bullet-list из 8 пунктов; каждый — 1-2 предложения + ref + tag `for further validation`:
  1. Encryption (AES-256-GCM at-rest+in-transit, key recovery) — ref `docs/competitive-landscape/DRAG_ANALYSIS.md` + WHITEPAPER §13
  2. ZK proofs (zkTAM-style embedding/retrieval correctness) — ref `docs/competitive-landscape/WEB_RESEARCH_TRUSTLESS_RAG.md` §4
  3. Shared namespaces multi-writer semantics — ref `docs/problems/CONCURRENT_WRITERS.md` + `docs/usecases/shared-project-memory-namespace.md`
  4. Reliability oracle — ref `docs/usecases/reliability-oracle-for-orchestration.md`
  5. Compressed shadow-index recall path — ref WHITEPAPER §4 + `docs/research/apply-to-agent-memory-architecture.md`
  6. Memory lifecycle policy / eviction — ref `docs/problems/MEMORY_EVICTION.md` + WHITEPAPER §13
  7. Economic model validation / Arweave pricing refresh — ref `docs/problems/ARWEAVE_PRICING_VALIDATION.md`
  8. Critical review redo against current Rust impl — ref upstream `sivo4kin/mnemonic-protocol:docs/CRITICAL_REVIEW.md@7a68a973` (not restored, original outdated)

**Audit trail**

- [ ] `recovered/README.md` обновлён: "Promoted on 2026-04-XX in commit <hash>"
- [ ] `recovered/` tree сохранён, не удалён

## Ограничения

- **Recovered/README.md классификация — авторитет** для verbatim vs validated vs archived, **с двумя override'ами владельца:** (i) `MCP_SERVER_BACKEND_FEATURES_COMPARISON.md` дропается полностью (не идёт в `historical/`); (ii) `CRITICAL_REVIEW.md` (не упомянут в README) дропается с follow-up bullet.
- **Delete-outdated, не rewrite (с минимальным override'ом).** Если sanity-grep находит stale claim — строка/секция удаляется. **Исключение:** если delete ломает таблицу или оставляет фразу без референта, разрешается **1-token replacement** (например `SHA3` → `blake3`). Каждый override логируется в `code-research.md` с before/after. Pre-flight уже зафиксировано **6 таких override'ов** в 3 файлах. Новые override'ы во время implementation допускаются по тем же правилам.
- **Нет код-изменений** в `core/`, `mcp/`, `webapp/`. Никаких `Cargo.toml`/`package.json` правок.
- **WHITEPAPER редактируется только §9** (расширение до 10 use cases). §10 и §13 — только если grep находит противоречие.
- **Source-of-truth**: локальный клон в `/Users/syi/src/mnemonic-protocol`, ветка `origin/docs/usecases` @ `7a68a973`. Pre-flight проверено: все 7 файлов лежат плоско в `docs/<filename>`.
- **Restoration mechanism**: `git -C /Users/syi/src/mnemonic-protocol show origin/docs/usecases:docs/<file> > .claude/skills/project-knowledge/recovered/<subdir>/<file>`. Upstream git history не сохраняется; attribution через commit-message с явным указанием `7a68a973`.
- **Out of scope**: `mnemonic-integrations`, `ai-tools-integration`, новые user-spec'и, удаление `recovered/`, IMPLEMENTATION_AUDIT.md/IMPLEMENTATION_STATUS.md из ветки `fix/restore-docs`.
- **Branch convention**: `feat/docs-actualization` from `dev`, PR back to `dev` (per `patterns.md`).
- **Conventional Commits**: `docs:`, `chore:`, `chore(pk):`, `chore(decisions):`.
- **CODEOWNERS** в репо нет — reviewers вручную не назначаются.

## Риски

- **R1: Scope creep в doc-rewrite.** Митигация: delete-outdated, не surgical rewrite; recovered/README.md — авторитет.
- **R2: Stale claim проскакивает мимо grep.** Митигация: список grep-терминов в `code-research.md` extensible; добавляем по ходу. Residual risk acceptable для doc-feature.
- **R3: WHITEPAPER §9 дублирует docs/usecases/.** Митигация: §9 = 1-2 предложения + ссылка (overview vs deep-dive).
- **R4: decisions.md follow-ups читаются как commited roadmap.** Митигация: explicit tag "for further validation" на bullet-items; только Browser-WASM имеет полную sub-секцию.
- **R5: sivo4kin upstream branch исчезает.** Митигация: используется локальный клон `/Users/syi/src/mnemonic-protocol`, ветка уже зафетчена; commit hash `7a68a973` в restoration-коммите.
- **R6: recovered/ удаляется преждевременно.** Митигация: явный acceptance criterion на retain.

## Технические решения

- **Restoration**: используется существующий локальный клон `/Users/syi/src/mnemonic-protocol`. Команда per file:
  ```bash
  cd /Users/syi/src/sessions/monorepo
  git -C /Users/syi/src/mnemonic-protocol show origin/docs/usecases:docs/DRAG_ANALYSIS.md \
    > .claude/skills/project-knowledge/recovered/competitive-landscape/DRAG_ANALYSIS.md
  ```
  Если перед запуском фичи локальный клон не на `origin/docs/usecases` — `git -C /Users/syi/src/mnemonic-protocol fetch origin` достаточно (ветка уже отслеживается).

  **Полный маппинг (11 файлов: 9 .md + 2 PDF; 1 файл upstream дропнут):**

  | upstream path | recovered/ destination | примечание |
  |----------|-----------|-----------|
  | `docs/DRAG_ANALYSIS.md` | `competitive-landscape/` | |
  | `docs/WEB_RESEARCH_TRUSTLESS_RAG.md` | `competitive-landscape/` | |
  | `docs/DECENTRALIZED_RAG_LANDSCAPE.md` | `competitive-landscape/` | field-landscape доку |
  | ~~`docs/MCP_SERVER_BACKEND_FEATURES_COMPARISON.md`~~ | — | **dropped** per owner decision (не восстанавливается, не архивируется) |
  | `docs/TURBOQUANT_DEEP_ANALYSIS.md` | `research/` | restore as-is с upstream recovery note про обрыв mid-Mermaid (181 строка) |
  | `docs/apply-to-agent-memory-architecture.md` | `research/` | |
  | `docs/condensed-principles.md` | `research/` | short TurboQuant principles, ссылается из project-knowledge |
  | `Agent Identity for Autonomous AI_ Protocols, Mnemonic Analysis, and the Path to a Minimal Primitive.pdf` (root) | `research/` | binary; sanity-grep skip |
  | `research/paper.pdf` | `research/paper.pdf` | foundational scientific paper; ref в WHITEPAPER §References + README; binary; sanity-grep skip |
  | `docs/MEMORY_EVICTION.md` | `problems/` (новая подпапка) | system problem statement |
  | `docs/CONCURRENT_WRITERS.md` | `problems/` | shared-context multi-agent problem |
  | `docs/ARWEAVE_PRICING_VALIDATION.md` | `problems/` | economic-model влияние |
- **Sanity-grep terms (initial set)**: `SHA3`, `mcp-server-rs`, `pre-V1 prototype`, `HashEmbedder`, `Python backend`, `Pre-V1, prototype validated`. Список ведётся в `code-research.md`, расширяется по ходу. PDF из `recovered/research/` исключается из grep'а как binary.
- **Drop rule**: ≥50% контента файла отмечен как stale ⇒ drop file целиком. ARCHIVED-классифицированные файлы (MCP_SERVER_BACKEND_FEATURES_COMPARISON.md) — exempt: идут в `docs/historical/` verbatim.
- **WHITEPAPER §9 формат**: per-use-case = `### 9.X <title>` + 1-2 предложения + `See [docs/usecases/<file>.md](./usecases/<file>.md) for the full pattern.`
- **decisions.md формат**: 1 sub-секция (Browser-WASM verification UI, ~20-30 строк), 6-item bullet-list (1-2 sentences each + source-doc ref + `for further validation` tag).
- **Commit plan** (13 атомарных коммитов в одной PR):
  1. `docs(recovered): restore competitive-landscape (3 files) from sivo4kin@7a68a973`
  2. `docs(recovered): restore research (3 .md + 2 PDFs) from sivo4kin@7a68a973`
  3. `docs(recovered): restore problem statements + pricing analysis from sivo4kin@7a68a973`
  4. `docs(recovered): extend recovered/README.md with problems/ + PDFs + drop notes`
  5. `chore(docs): sanity-grep report → code-research.md (incl. 6 token-replace overrides)`
  6. `docs: promote recovered/usecases → docs/usecases/`
  7. `docs: promote recovered/competitive-landscape → docs/competitive-landscape/ (apply 4 token replaces)`
  8. `docs: promote recovered/research → docs/research/ (incl. 2 PDFs)`
  9. `docs: promote recovered/problems → docs/problems/ (apply 2 token replaces in CONCURRENT_WRITERS)`
  10. `docs(whitepaper): expand §9 use cases (all 10) + reference foundational paper in §References`
  11. `docs(readme): reference foundational paper`
  12. `chore(pk): update project-knowledge references`
  13. `chore(decisions): add follow-up roadmap for docs-actualization`

## Тестирование

**Без code-tests** — pure documentation feature.

**Automated gates (CI на PR):**

- `lychee --offline docs/` — gating, ноль битых internal-ссылок
- `markdownlint` на изменённых .md — advisory (не блокирующий, если в репо нет конфига)
- cargo CI **не запускается** — paths-ignore покрывает docs-only changes (per `.github/workflows/ci.yml` после dd395fd)

**Manual gates (локально / на ревью):**

- Re-run sanity-grep против актуальных `core/` + `mcp/`; обновлённый `code-research.md` — в коммит, если новый hit
- Spot-read: WHITEPAPER §9 expansion, PK edits, decisions.md follow-ups, recovered/README.md promotion-нота
- Per-file content review `docs/usecases/`, `docs/competitive-landscape/`, `docs/research/` — **пропускается** (delete-outdated trust policy)

**Post-merge:**

- Cloudflare Pages docs project auto-rebuilds (per deployment.md); spot-check landing.

## Как проверить

### Агент проверяет

| Шаг | Инструмент | Ожидаемый результат |
|-----|-----------|---------------------|
| 1. Restoration files exist | `bash: ls .claude/skills/project-knowledge/recovered/competitive-landscape/*.md .claude/skills/project-knowledge/recovered/research/*.md .claude/skills/project-knowledge/recovered/research/*.pdf .claude/skills/project-knowledge/recovered/problems/*.md` | 3 + 3 + 2 + 3 файла |
| 2. lychee link check | `bash: lychee --offline docs/` | exit 0 |
| 3. Stale-term grep | `bash: grep -RIE 'SHA3\|mcp-server-rs\|pre-V1\|Pre-V1\|HashEmbedder\|Python backend' docs/` (`-I` skip binaries) | ноль hits |
| 4. docs/ layout | `bash: ls docs/usecases/ docs/competitive-landscape/ docs/research/ docs/problems/` | 11 / 4 (3 + README) / 6 (3 .md + 2 PDF + README) / 4 (3 + README) |
| 4z. historical/ NOT created | `bash: test ! -d docs/historical || echo "FAIL: docs/historical exists"` | пустой stdout |
| 4a. WHITEPAPER refs paper.pdf | `bash: grep 'docs/research/paper.pdf' docs/WHITEPAPER.md` | непустой (в §References) |
| 4b. README refs paper.pdf | `bash: grep 'docs/research/paper.pdf' README.md` | непустой |
| 5. WHITEPAPER §9 | `bash: grep -E '^### 9\.' docs/WHITEPAPER.md \| wc -l` | >= 10 |
| 6. PK project.md updated | `bash: grep -E 'Use Case Roles\|docs/usecases' .claude/skills/project-knowledge/references/project.md` | непустой |
| 7. PK architecture.md updated | `bash: grep 'docs/competitive-landscape' .claude/skills/project-knowledge/references/architecture.md` | непустой |
| 8. decisions.md follow-ups | `bash: grep -E 'Follow-up roadmap items\|Browser-WASM verification UI\|for further validation' work/docs-actualization/decisions.md` | все 3 строки найдены |
| 9. code-research.md | `bash: test -f work/docs-actualization/code-research.md` | существует |
| 10. recovered/ retained with note | `bash: grep 'Promoted' .claude/skills/project-knowledge/recovered/README.md` | непустой |

### Пользователь проверяет

- Открыть PR на GitHub: убедиться что diff в `core/`, `mcp/`, `webapp/`, `Cargo.toml`, `package.json` — пустой
- Открыть `docs/WHITEPAPER.md` §9 — все 10 use cases читаются как short overview с рабочими ссылками на `docs/usecases/<file>.md`
- Открыть `work/docs-actualization/decisions.md` — Browser-WASM sub-секция читается как seed для будущего user-spec; bullet-list из 6 — короткий и помеченный "for further validation"
- Опционально: `cd /tmp && git clone <PR branch> && cd <repo> && lychee --offline docs/` — gate проходит локально
