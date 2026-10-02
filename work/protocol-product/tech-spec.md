# Product protocol technical specification

Status: planned. MUST and MUST NOT define target requirements, not shipped behavior.
Reviewed baseline: `89532db4556e53e81b5243c57535a76046c08207`.
See [baseline audit](baseline-audit.md) for current implementation and test limits.

## 1. Scope

Provide artifact delivery and client recovery across operator database loss and runtime migration.
Preserve existing signed formats and verification rules.
This specification adds service contracts around existing formats.
It does not replace [A2A recovery](../sealed-memories/a2a-recovery-spec.md).

The first backend remains the repository's supported Arweave/Irys path.
Backend independence is an interface goal. It is not proof that every provider is supported.
Each adapter MUST declare fetch, discovery, size, consistency and retention capabilities.
Unsupported discovery MUST be explicit. Known-locator recovery can still work separately.

## 2. Client artifact preparation

The target private path MUST construct, encrypt and sign on the client.
The operator MUST NOT receive plaintext, content keys or a recall key.
The client MUST retain original signed bytes until delivery resolves.
Public mode MUST require explicit disclosure consent before permanent upload.
Local mode MUST work on the agent's device without hosted API, payment or gateway access.
Hosted API local mode MUST fail before payment or upload.

Legacy server-prepared signing is a migration path, not the target privacy guarantee.
Its documentation MUST state what the server sees and constructs.
Do not call noncustodial signing proof of client-only artifact preparation.

## 3. Ingestion and operation identity

Proposed logical request fields:

| Field | Meaning |
|---|---|
| `artifact_bytes` | Original complete signed envelope; binary transport encoding is chosen during task 2 |
| `expected_author` | Caller-bound identity, checked against verified signature |
| `mode` | `anchored`; local writes do not call hosted ingestion |
| `artifact_kind` | Supported memory or A2A kind; verified against envelope |
| `operation_id` | Stable retry identity scoped to payer/operator |
| `quote_id` | Server quote bound to digest, size, backend, amount and expiry |
| `prev_locator` | A2A parent fetch hint under the existing recovery contract |

This table is an interface proposal, not a new deployed endpoint.
Task 2 MUST choose one canonical transport and adapt current endpoints.
All adapters MUST reach shared validation, quota, payment and delivery logic.

Compute transport digest over original envelope bytes.
Keep that digest separate from memory content hash and A2A binding hash.
Use the existing A2A binding hash for `prev_id` and A2A identity.
Do not redefine signed field order, `kid`, SEALED_V1 or GRANT_V1.

Validate size, mode, author, envelope, signed bindings and parent before irreversible payment or upload where possible.
Never use a receipt row to bypass cryptographic validation.
Reject reused operation IDs with different bytes or quote bindings.
Same-artifact retries MUST preserve their financial operation identity.
Concurrent retries MUST not create a second charge for the same accepted operation.
This guarantee depends on durable financial replay state.

## 4. Delivery and evidence

The operator MUST upload the original signed bytes unchanged.
It MUST fetch the resulting locator through a bounded configured gateway.
It MUST compare exact bytes and verify the expected author and artifact identity.
Only that result can set delivery to `verified`.
SQL persistence is not part of cryptographic delivery verification.
A receipt failure after verified delivery MUST retain the external success distinction.

Proposed receipt fields:

| Field | Meaning |
|---|---|
| `operation_id`, `artifact_kind` | Request identity and verified kind |
| `author`, `artifact_id` | Verified author and existing format identity |
| `envelope_digest`, `byte_length` | Exact delivered-byte commitment |
| `backend`, `locators` | Fetch method and one or more locations |
| `delivery_status`, `verified_at` | Operator's bounded delivery observation |
| `payment_status`, `payment_reference` | Separate financial result |
| `retry_action`, `error_code` | Safe next action and structured failure |

The client MUST independently fetch and verify receipt claims before final import.
A successful read proves availability at that observation time only.
Longer availability claims require monitoring, measurement and explicit service terms.
Multiple locators may identify the same artifact after operator rotation.

## 5. Separate payment and delivery states

| Delivery state | Meaning |
|---|---|
| `validated` | Input passed required checks |
| `uploading` | External delivery is in progress |
| `verification_pending` | A locator exists; verified fetch has not completed |
| `verified` | Original bytes passed bounded fetch and verification |
| `failed_retryable` | Another attempt may resolve delivery |
| `failed_terminal` | Current operation cannot deliver its accepted request |

| Payment state | Meaning |
|---|---|
| `not_required` | Free or quota-backed operation |
| `required` | Valid quote awaits accepted payment |
| `authorized` | Rail permits later settlement, if supported |
| `settled` | Funds moved; delivery may still be pending |
| `remedy_pending` | Refund or credit is owed under stated policy |
| `remedied` | Remedy completed with evidence |

Not every rail supports authorization before settlement.
Document actual ordering for each supported rail.
Do not promise universal payment after delivery.
Expose a remedy policy for settled operations with terminal delivery failure.
Validate invalid requests before settlement where the rail permits this ordering.
Financial amounts and policy remain in `payment.rs` and `pricing.rs`.

## 6. Retry without hosted memory storage

The target operator database MUST NOT retain artifact bytes or grant blobs.
The baseline retry design requires client resubmission of identical signed bytes.
Bind resubmission to the operation's stored digest and financial state.
Persist locator and verification progress when available.
After a crash, recheck known locators before upload or settlement.
Do not replace missing source bytes with reconstructed or newly signed bytes.

Autonomous retry requires a separate, approved temporary staging policy.
That alternative needs a duration, deletion rule, access boundary and legal review.
It is not authorized by this baseline contract.
Keep OAuth, accounting and replay records durable under their own retention policy.

## 7. Discovery, recovery and completeness

A2A MUST follow the existing recovery specification and tasks 18–21.
General memory recovery MUST preserve source errors instead of returning silent empty success.
Dispatch reconstruction by verified artifact kind, including supported sealed artifacts.
Unsupported or malformed kinds MUST produce structured diagnostics.

Proposed recovery result dimensions are independent:

| Dimension | Required result |
|---|---|
| Source scan | Exhausted, partial, failed or budget exhausted; source diagnostics included |
| Candidate verification | Imported and rejected counts with reasons |
| Graph | Forks, missing parents and invalid edges where relevant |
| Completeness | `complete_to_heads` or `unknown` relative to pinned heads |
| Open | Verified complete plaintext or explicit failure; never partial success |

Top-k recall is not enumeration.
An inclusion proof is not a proof of absence, correct ranking or complete history.
SQL Merkle roots cannot establish independent owner commitments.
Future committed index proofs need authenticated scope, root and freshness rules.
Do not include those claims in the first product release.

## 8. Recovery checkpoints

Planned client checkpoint fields: version, artifact kind, context or corpus scope,
expected authors, expected head IDs, known locators, backend hints and creation time.
The client signs the checkpoint or authenticates it through an independent trusted backup.
The recovery caller MUST pin its trust anchor outside the fetched artifact set.
Timestamp is descriptive; it does not establish consensus or newest-head status.

Task 4 defines canonical checkpoint encoding and compatibility tests before release.
No checkpoint can reveal an unrecorded later head.
No checkpoint replaces backup of required decryption keys.
Loss of a decryption key can leave intact ciphertext unreadable.

## 9. Release boundaries

Each capability MUST declare local, hosted, memory-kind and backend support.
Do not infer production readiness from a unit test or a merged spec.
Use [acceptance.md](acceptance.md) for release claims and [gaps.md](gaps.md) for closure evidence.
Keep public tools, SDK documentation and discovery cards consistent with released behavior.

## 10. Recovery and development follow-up

The agreed goal includes independence from the operator and supported storage backend.
Use [storage-portability-spec.md](storage-portability-spec.md) for conditions and migration acceptance.
Use [indexer-spec.md](indexer-spec.md) for replaceable discovery.
Use [flows.md](flows.md) for logical information and payment sequence.
Use [shipping-plan.md](shipping-plan.md) for release gates.
Repository extraction remains a proposal under [repository-plan.md](repository-plan.md).
These contracts do not modify existing signed formats.
