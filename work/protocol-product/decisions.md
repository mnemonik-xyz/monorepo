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

## 2026-10-02 — Task 9 replaceable discovery

Implemented SDK discovery interfaces and Irys/Arweave adapters with source-bound continuation and explicit disabled-discovery recovery.
Source diagnostics preserve outage, malformed responses, budgets and cancellation instead of presenting empty success.
Artifact signatures and pinned authors retain authority; changing the index cannot add trusted identities.
Root review added credential omission, redirect refusal and deadlines spanning response-body reads.

Checks: 21 SDK discovery/recovery tests passed with rebuilt real WebAssembly and mocked external sources.
`npx tsc --noEmit -p packages/sdk/tsconfig.json` passed.
Tests include replacement, reordered duplicates, competing roots, omission, lag, forged hints, cursor loops and known-locator bypass.
Task 9 closes its implementation acceptance. Live-provider claims remain gated by task 5 and A-13.

## 2026-10-02 — Task 4 recovery and authenticated backup

Implemented source diagnostics with retained partial pages and explicit scan budgets.
The local restore command now reports incomplete discovery and uses the correct Irys owner filter.
Added native verified-kind recovery, exact-envelope retention, complete plaintext and structured unsupported/key errors.
SDK checkpoints authenticate scope, identity, heads and locators; portable backups encrypt explicit caller-owned key material.

Root review found that memory IDs alone cannot pin legacy memory bytes.
The native graph therefore requires independently authenticated author/digest bindings for heads and every ancestor.
Cross-review also aligned existing SDK JSON plaintext with native CBOR recovery without rewriting signed formats.
Checkpoint verification snapshots caller data before asynchronous verification; multi-locator selection cannot silently discard alternatives.

Checks passed: 15 native recovery/restore tests, 11 GraphQL tests, one memo enumeration test and seven SDK checkpoint tests.
SDK TypeScript checking and strict core-library clippy passed.
Task 4 closes implementation acceptance for declared interfaces. Live providers and released-package recovery remain task 5 gates.

## 2026-10-02 — Parallel implementation readiness

Tasks 2 and 3 share a final metadata-only acceptance condition.
Task 3 may start after task 2's shared ingestion interface passes focused review and tests.
Task 2's final paid-path and storage acceptance stays open until task 3 integration passes.
This distinction avoids a circular acceptance gate without declaring either implementation complete early.
Task 10 starts after the ingestion interface, recovery/checkpoints and discovery interfaces are available and tested.
It must still pass integrated migration and continuation tests before closure.


## 2026-10-02 — Task 10 bounded storage portability

Owner-signed locator manifests bind routing alternatives to authenticated checkpoints.
The SDK copies exact original bytes and validates destination-only pinned ancestry.
The MCP resolver accepts blob digests only through an explicitly configured origin.
Independent operator continuation was tested after source shutdown and receipt deletion.

Checks passed: five SDK storage tests, seven checkpoint tests, one native resolver test, and the MCP independent-operator continuation test.
The latest full SDK run passed 365 tests across 29 files.
This closes task 10's declared A2A implementation scope, including embedded grants and completed streams.
The [evidence matrix](implementation-evidence.md) separates these mocked services from live evidence.
General-memory migration and standalone grant/SSE migration are not supported by this interface.
No production backend, permanent retention or published-package readiness is claimed.


## 2026-10-02 — Integrated implementation and task 2/3 acceptance

Implementation commit `9fc8e42` contains shared client-prepared ingestion, metadata-only new retry records, authenticated recovery, replaceable discovery and bounded A2A storage migration.
Three agents worked on independent areas; root review integrated payment, recovery, CLI and acceptance evidence.
Cross-review fixed quota cancellation state, provider/cached receipt bindings, checkpoint mutation, memory identity pins, storage budgets and CLI durability/private-query handling.

Final checks passed: 1,305 Rust workspace tests (seven ignored); 365 SDK tests; 190 CLI tests (two skipped).
The normally ignored real SDK/WASM HTTP checks ran explicitly and passed: two signed-memory/recall tests and one A2A SQL-loss recovery test.
SDK/CLI builds, strict core/MCP Clippy and local documentation checks passed.
See the [implementation evidence matrix](implementation-evidence.md) for exact commands and limits.

Tasks 2 and 3 close implementation acceptance for their declared interfaces.
Tasks 4, 9 and 10 have implementation evidence in the same commit; task 11 remains a planning-only completion.
Historical paid staging is deliberately retained until identical verified resubmission drains it.
The configured payment provider remains a trust anchor; no live settlement or detached provider-signature verification is claimed.

Task 5 remains in progress: mocked operators and storage do not establish live discovery, deployed independent-operator support or a released-package drill.
Tasks 6, 8 and 12 retain their release prerequisites. Accurate source documentation and a capability matrix were updated now to prevent stale claims.
Task 7 has no customer evidence or outreach; no product demand is inferred from these tests.
No merge, deployment, package publication or repository extraction has been performed.


## 2026-10-02 — Task 5 evidence and README values

After merge `7dc5079`, three agents independently audited A2A acceptance, strengthened the operator-switch drill, investigated build failures and rewrote the README.
The README now leads with client-private preparation, verifiable authorship, client-owned copies and an exit path. Source capabilities remain distinct from hosted/released support.
The documented standalone CLI init/sign/open sequence passed using a temporary file-backed identity.

Commit `98d18d0` adds a combined real SDK/WASM HTTP drill with signed checkpoints/manifests, exact-envelope comparisons, sealed streams/grants, source/operator shutdown, SQL loss and fresh-recipient continuation through independently keyed O2.
Actual two-operator uploads deduplicate to the same signed artifact; invalid parent variants produce no uploads or financial operations.
The evidence exposed and fixed HTTP prevalidation that mislabeled a retriable parent outage as invalid input.
All 11 tests passed after correction; strict MCP Clippy passed.
The enumeration example now rejects an empty memo comparison as inconclusive, verified with local mock GraphQL/RPC services.

Commit `6301dea` corrects missing conformance lock entries and missing Docker workspace members, both predating PR #267.
Clean isolated webapp install/build and 16 sealed-view tests passed; isolated Docker-context Cargo metadata passed.
Pinned published WASM still lacks the sealed opener; the page reports unsupported crypto rather than bypassing checks.
Cloudflare dashboard logs were not available, so its exact build failure cause is not inferred from the GitHub workflow failures.

Live read-only observations found 24 Irys App-Name candidates but zero A2A candidates; the Arweave index returned zero for both filters.
The [release-evidence record](release-evidence.md) preserves endpoints, timestamps, scan bounds and the JSON observation record.
Exit 2 from the probe explicitly means no positive pinned A2A fixture was established.
Task 5 and existing A2A acceptance remain in progress. Known synthetic fixture pins, submission time/lag observations and configured live operator evidence are still required.
No payment, deployment, package publication, customer outreach or issue closure was performed.

## 2026-10-02 — Source-candidate release preparation

Following merged PR #268 (`26a9550`), tasks 6, 8 and 12 move to in-progress
preparation without changing dependencies or declaring release acceptance.
Accurate source documentation, a repeatable local demonstration and a runbook
can advance while task 5's positive live evidence remains open.

The capability matrix separates source support from deployed/published support.
The research handoff integration binary passed 11 tests; its static report viewer
passed three tests and a Chrome visual check. The isolated candidate tarball
probe passed install, standalone identity, sealed save and fresh-process open.
Its recorded hashes were checked before archiving the report. These are
unpublished prebuilt candidates: checkout identity does not establish build
provenance, and local save/open does not establish hosted recovery or A-17.

The CI image build for `26a9550` succeeded and published manifest-list digest
`sha256:113a30acc0905ea1b1af36b06521540981d0b196eae078dde102a8ead6099348`.
Build publication is distinct from deployment. The runbook preserves existing
fabric/Caddy ownership and financial replay/key/checkpoint data across rollback.
See [release evidence](release-evidence.md) for commands, reports and limitations.
No implementation commit is assigned to this preparation before it is committed.
No deployment, npm publication, live payment or customer outreach was performed.

Preparation implementation: `355fa84` (research handoff, recorded evidence viewer, isolated package drill). Documentation in the following commit records the remaining release gates.
