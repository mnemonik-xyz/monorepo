# Artifact payment and retry behavior

Available now: signed-memory ingestion and HTTP A2A ingestion share durable delivery operations.
The supported paid path uses the existing Universal Paywall exact rail.
Other payment rails fail closed on this shared path.
Legacy deferred-sign callbacks remain a migration interface.

## Client-owned retries

Keep the original signed envelope until delivery resolves.
Resubmit those exact bytes with the same `x-mnemonic-operation-id` header.
If omitted, the server derives a stable operation ID from the author and envelope digest.
The SDK accepts `operationId` in `ingestPreparedMemory` options.
Its structured error cause preserves the response, operation ID, and payment challenge.

The server binds the operation to its author, digest, size, backend, and deterministic locator before payment.
Changing any bound field produces a conflict.
A durable lease serializes concurrent payment and upload attempts.
An expired lease permits recovery of the same financial operation.
The locator exists in metadata before upload starts.
After a crash, the server checks that location before uploading again.

A request without sufficient free quota first requires a linked wallet.
The response identifies the operation and wallet-link challenge.
After linking, resubmit the original bytes to obtain the immutable quote.
Payment authorization settles that quote before delivery.
Retries reuse the durable provider receipt and never create another charge for the accepted operation.
The server validates quote and receipt fields against the durable operation before accepting settled evidence.
Malformed historical receipts require reconciliation.
The configured payment provider remains the settlement trust anchor.
This guarantee requires durable financial replay records and the provider's operation idempotency.
Losing those records is a reconciliation incident, not authorization to charge again.

## Independent outcomes

Receipts expose `payment_status` and `delivery_status` separately.
A settled payment can still have retryable delivery failure.
Exact fetched bytes establish `verified` delivery independently of SQL receipt writes.
`receipt_persisted` and `operation_persisted` report metadata persistence separately.
A later successful retry reconciles available operation metadata.
Quota cancellation resets payment eligibility before refunding the reservation.
If metadata cannot be reset, the reservation stays consumed so a retry cannot bypass payment.
If financial metadata is unavailable on a paid deployment, external success reports payment status `unknown` and requests financial reconciliation.

Eight failed paid upload attempts produce terminal delivery failure and `remedy_pending`.
The operator must review delivery completion, a refund, or a documented service credit.
No retry worker automatically transfers funds or grants credit.
A remedy becomes `remedied` only after an operator records its evidence reference.
The existing [operations runbook](../work/universal-paywall-integration/staging-operations.md) defines review ownership and evidence.

## Upgrade retention boundary

New operations retain financial state, digests, locators, and receipt metadata only.
New legacy-callback staging rows contain no COSE, canonical payload, content, embedding, or application tags.
Callbacks reconstruct transient context from the client's identical signed resubmission.
The background upload worker no longer consumes hosted artifact bytes.

Historical staging rows are an explicit upgrade exception.
Boot migration preserves them to avoid destroying previously paid operations' only retained delivery source.
Startup reports their count without logging their content.
Identical verified client resubmission drains the old payload columns.
Changed-byte retries cannot drain or replace another operation.
Operators must finish this drain before claiming the database contains no historical artifact bytes.
Database backups, journals, and storage retention need their own operational disposal policy.
Do not delete undelivered historical bytes merely to clear the count.

## Verification limits

Tests cover concurrent leases, settlement crashes, upload crashes, changed-byte replay, receipt failure, and terminal remedy visibility.
HTTP tests use real signed memory and A2A artifacts with mocked storage.
Financial-crash fixtures seed an accepted durable receipt; they do not execute a live payment.
No live settlement, refund, or provider availability claim follows from these tests.
