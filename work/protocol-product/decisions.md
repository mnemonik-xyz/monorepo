# Product protocol decisions

Append-only. Status changes require a new dated entry.

## 2026-10-01 — Owner inputs

- No current goal to move to a native token.
- Use the signed-memory delivery and recovery service definition in README.
- Reconsider protocol scope, layers, modules, value, buyer and sales model.
- Check the repository and record discrepancies before proposing changes.
- Prepare specifications and documentation for the discussion.

## 2026-10-01 — Proposed implementation decisions

| ID | Decision | Reason | Status |
|---|---|---|---|
| P-1 | Client builds and signs private artifacts | Hosted operator needs no plaintext or content key | Proposed |
| P-2 | Hosted database keeps utility and receipt metadata only | Delivered memory survives receipt loss | Proposed; A2A already specified |
| P-3 | Client resubmits exact bytes for retries | Avoid durable hosted artifact staging | Proposed |
| P-4 | Payment and delivery have separate states | Payment ordering differs by rail | Proposed |
| P-5 | Recovery uses independent trust anchors and pinned heads | Unsigned discovery cannot prove complete history | Proposed; A2A already specified |
| P-6 | Sell managed execution and continuity | Open format alone does not justify a fee | Buyer hypothesis |
| P-7 | Keep existing A2A tasks 18–21 authoritative | Avoid duplicated implementation ownership | Documentation organization |

## 2026-10-01 — Open product choices

| Choice | Default for specification | What resolves it |
|---|---|---|
| First customer integration | One long-lived agent runtime | Design-partner evidence |
| Pricing | Usage plus external cost; team plan experiment | Measured costs and willingness to pay |
| Temporary staging | Disabled; require client resubmission | Explicit retention decision and counsel if later required |
| Checkpoint encoding | Separate versioned client object | Task 4 design and compatibility review |
| Availability target | No numeric agreement yet | Sustained measurements and support capacity |
| Legal entity and fundraising | Unresolved | Owner budget and counsel |

No recommendation here authorizes token issuance, a treasury transaction or customer legal terms.
Documentation acceptance does not mark implementation shipped.

## 2026-10-01 — Multi-agent continuity demonstration

The owner requested a proof of concept with real-life roles and flow.
Proposed scenario: research handoff across independently operated agents.
The claim is verifiable memory provenance and continuity, with client-held keys.
It does not claim truthful reasoning, undisclosed-history completeness or provider-free availability.
See [proof-of-concept.md](proof-of-concept.md) and [task 8](tasks/8.md).
This is a proposed demonstration choice; no passing implementation is reported.

## 2026-10-02 — Recovery goal and architecture follow-up

**Agreed owner goal:** fully restore memory independently of the chosen operator and supported storage backend.
Full recovery means verified required artifacts and ancestry up to an independently preserved checkpoint.
It requires retained keys, trusted identities and accessible original bytes.
Backend replacement requires migration or access to prior copies.

**Proposed indexer boundary:** discovery is a separate logical role.
The backend index may implement it first.
Separate deployment is optional. Index results never establish artifact authority or completeness.

**Proposed repository direction:** extract the canonical protocol contract first.
Keep coupled implementation together initially.
Other repository names and extraction actions remain proposals.

The owner requested Markdown specifications and a GitHub push.
This authorizes documentation work in the existing repository.
It does not authorize creating repositories, changing runtime behavior or deploying services.

New contracts: [indexer](indexer-spec.md), [portability](storage-portability-spec.md),
[repository plan](repository-plan.md), [flows](flows.md) and [shipping](shipping-plan.md).
Tasks 9–12 cover implementation and planning follow-up.
Existing A2A tasks 18–21 remain authoritative for their signed bindings and recovery paths.
No implementation gap or product-demand hypothesis closes from this documentation change.

## 2026-10-02 — Task 1 baseline and claim map

Reviewed source revision `89532db4556e53e81b5243c57535a76046c08207`, including implementation #265 and planning #266.
Three parallel source audits covered ingestion, A2A recovery, payment and decoupling.
The [baseline audit](baseline-audit.md) maps F1–F13 and all seventeen acceptance claims to current paths or remaining tasks.

F6's former operator-signing and SQL-artifact description is superseded by `979c590`.
Existing sealed tasks 18–21 remain in progress; their merged implementation does not complete acceptance.
Other findings remain open. General delivery still depends on SQL, and paid retries retain content and signed bytes.
The direct sealed anchor route bypasses shared gates and still submits a Solana memo.
Ordinary memo removal therefore does not prove universal removal or a passed enumeration gate.

Checks: a local path checker validated Markdown targets in this package and the edited decoupling/A2A task records.
A dependency traversal validated all twelve product tasks without cycles. `git diff --check` passed.
Test references were inspected, not executed. No live-source, production, payment or customer evidence was generated.
Task 1 closes its audit scope only. No implementation finding, provider claim or product hypothesis closes.
The implementation source revision above is the evidence baseline; this audit adds no runtime implementation.

Tasks 2, 4, 9 and 11 can now proceed with explicit file ownership.
Task 7 can prepare research, but contacting people still requires explicit outreach authorization.

## 2026-10-02 — Task 11 repository extraction proposal

The [repository inventory](repository-inventory.md) completes the requested planning deliverable at source baseline `89532db`.
It inventories four Rust crates and six npm manifests, with source versions and local build dependencies.
It maps specification ownership, vectors, relative links, release jobs and registry metadata to required migration work.
It proposes compatibility manifests and extraction gates E1–E6.

Checks: inspected manifests, build scripts and release workflows; local Markdown target checks and `git diff --check` passed.
No runtime tests were needed for this planning change. No registry publication status was inferred from source versions.
Task 11 closes planning only. Repository names, ownership and extraction remain proposals.
No repository, deployment, package publication or code move was performed.
