# Recovery and storage portability

Status: agreed recovery goal with planned implementation requirements.

## Goal

Fully restore memory independently of the chosen operator and supported storage backend.
Full recovery covers required artifacts and signed relationships up to an independently saved checkpoint.
The client verifies authorship, integrity, grants and ancestry, then opens complete authorized content.

Required conditions: preserved keys, trusted author identities, authenticated checkpoint and accessible original bytes.
This goal does not restore tool credentials, unfinished operations or all external effects.
It does not promise recovery of unanchored data lost from the only local copy.

## Separate independence properties

| Property | Required demonstration |
|---|---|
| Operator independence | Restore with original operator disabled and its memory receipts deleted |
| Indexer independence | Replace discovery source or bypass it through known locators |
| Backend compatibility | Fetch and verify through every advertised supported adapter |
| Backend migration | Copy exact signed bytes to another supported backend and recover there |
| Identity continuity | Preserve artifact identity, signatures, grants and parent relationships across copies |

Supporting one backend through an adapter does not prove cross-backend migration.
Changing an endpoint does not move existing data.
Keep access to old copies until the destination passes verification.

## Capability contract

Each supported adapter declares upload, known-locator fetch and discovery support.
Also declare size limits, retention model, consistency behavior and relevant metadata exposure.
Reject unsupported operations explicitly.
A backend without enumeration may support known-locator recovery only.

Use a locator containing an explicit backend namespace and backend address.
Canonical locator encoding is implementation work under task 10.
Preserve existing ar:// addresses and signed formats.
A locator change must not redefine content identity or signed parent references.

## Migration flow

1. Load the independent checkpoint and trusted author set.
2. Fetch required original bytes through a supported source.
3. Verify signatures, artifact identities, grants and required graph edges.
4. Copy exact original signed bytes to the destination.
5. Fetch destination copies and compare exact bytes and verified identities.
6. Build a client-owned locator manifest for verified destination copies.
7. Independently back up that manifest and checkpoint.
8. Restore with the original operator and source backend unavailable.

An optional migration service receives signed bytes and routing hints, not decryption keys.
Its transport signature never replaces the artifact author signature.
Account for destination fees through the existing payment contract.

## Locator manifest and trust

The manifest maps verified artifact identity to one or more backend locators.
Authenticate it through the client checkpoint or an independently trusted backup.
It is a routing aid; verify every fetched artifact.
Do not require an operator SQL row to resolve existing artifacts.
Do not rewrite existing signed parent bindings during migration.

Where the existing format lacks sufficient portable routing, record that limitation.
Provide verified external hints rather than silently changing legacy signed bytes.
Version any new manifest schema before release.

## Acceptance and limits

Test two declared adapter implementations, exact-byte migration and source shutdown.
Test sealed artifacts, grants, complete streams, forks and missing parents.
Check recovery and continuation after receipt loss.
Separate local fixtures from live production backend evidence.
Do not advertise provider-independent availability from two adapter mocks.
See [task 10](tasks/10.md).
