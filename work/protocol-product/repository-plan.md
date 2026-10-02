# Repository boundaries and extraction plan

Status: proposal. The owner requested discussion, not creation of new repositories.

The [inventory and migration proposal](repository-inventory.md) records task 11's reviewed baseline and extraction gates.

## Recommendation

Extract the protocol contract first.
Keep coupled implementation and end-to-end tests together while existing gaps close.
A logical role does not automatically need a separate repository.

| Proposed repository | Scope | Trigger |
|---|---|---|
| mnemonik-protocol | Normative formats, invariants, vectors and conformance requirements | Reviewed canonical specification and link migration plan |
| Current monorepo | Core, SDK, CLI, operator and integrated tests | Remains the implementation home initially |
| mnemonik-indexer | Independent discovery implementation and source adapters | Independent service and release need |
| mnemonik-demo | Reproducible handoff, recovery and operator-switch demonstration | Stable demo dependency versions |
| mnemonik-web | Website and product interface | Independent release cycle |
| Future core and operator repositories | Published verifier library and hosted service | Stable interfaces and independent maintainers or releases |

Names and extraction remain proposed.
Keep specifications canonical in one place.
Do not create two editable normative copies.

## Dependency direction

The protocol contract defines conformance.
Core implements portable encoding and verification.
SDK and operator consume core.
Indexer implements the discovery contract.
Demo consumes published or pinned compatible implementations.

Core must not depend on operator, payment policy or hosted SQL.
Adapters must not alter signed format identity.
Payment policy remains outside portable core.

## Extraction prerequisites

Inventory package dependencies, local paths, workspace builds and release automation.
Define protocol and package version compatibility.
Publish common vectors and a reusable conformance suite.
Pin dependency versions and artifact checksums where applicable.
Create an integration job covering supported component combinations.

Record migration of source links, issue references, documentation links and ownership.
Preserve history where feasible and keep a contributor migration guide.
Choose visibility, license continuity and publishing credentials explicitly.
Do not migrate secrets into documentation.

## Sequence

1. Inventory boundaries and produce a reviewable extraction proposal.
2. Agree canonical ownership and package release rules.
3. Extract specification and conformance assets.
4. Repair links and verify consumers against the extracted contract.
5. Extract other components only when their release needs justify it.

Implementation gap closure can proceed before repository extraction.
The planning task does not authorize repository creation or moving code.
See [task 11](tasks/11.md).
