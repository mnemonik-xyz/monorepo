# Discovery indexer contract

Status: planned interface. A separate deployed indexer is not required for the first implementation.

## Role

The storage backend holds original signed bytes.
The operator accepts and delivers artifacts.
The discovery indexer finds candidate locators.
The client verifies candidates, opens authorized memory and builds its local recall index.

Backend indexing is a valid first implementation of this role.
Independent indexers can replace it through the same interface.
An indexer is not an artifact authority, author registry or decryption service.
Do not store plaintext, private embeddings, keys or access credentials in a discovery index.

## Proposed interface

| Field | Requirement |
|---|---|
| Request scope | Artifact kind, expected authors and context or supported corpus scope |
| Endpoint configuration | Explicit index flavour, endpoint and supported backends |
| Pagination | Opaque cursor; bounded page and total candidate budgets |
| Candidate | Backend identifier, locator and optional discovery metadata |
| Source status | Exhausted, partial, unavailable, malformed or budget exhausted |
| Continuation | Resume token bound to query scope and endpoint configuration |
| Diagnostics | Source errors, rejected candidates, cursor loops and cancellation |

This is a logical interface, not a deployed API.
Reuse the existing [A2A discovery contract](../sealed-memories/a2a-recovery-spec.md).
Use its provider-specific pagination and bounded fetching requirements.
Do not replace its existing author or hash semantics.

Discovery metadata and tags are hints.
The client verifies original signed bytes against independently trusted authors.
Indexer timestamps are not consensus ordering or authenticated freshness.
Deduplicate verified artifact identity across candidate locators.
Preserve valid forks.

## Recovery paths

Known-locator recovery can bypass the indexer.
Context discovery requires an index or another explicit enumeration source.
A missing result may mean lag, omission or provider failure.
Never convert provider failure into empty success.
A completed scan does not prove that no undisclosed artifact exists.

The client checks ancestry against an independently authenticated checkpoint.
Missing required parents prevent a complete-to-heads result.
An authenticated checkpoint is not proof that its signer disclosed every later head.

## Replacement and privacy

Switching indexers must not change signed artifact formats.
Cursor formats may differ; restart or resume only with a compatible source.
Retain verified local entries when a replacement source returns partial results.
Disclose public author, context, timing and routing metadata before upload.
Encryption of content does not hide that metadata.

## Acceptance

Use two index implementations over the same fixture.
Confirm identical verified identity despite different ordering and duplicate locators.
Test omissions, lag, forged tags, cursor loops, outage and scan budgets.
Confirm known-locator restore works with both indexes disabled.
Confirm source replacement never grants new author trust.
See [task 9](tasks/9.md).
