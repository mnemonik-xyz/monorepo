# Artifact Attestation Service

## Pattern summary

Use Mnemonic to attest, index, and retrieve artifacts produced by A2A workflows.

Artifacts can include:

- reports
- code patches
- evidence bundles
- recommendations
- claim sets
- structured outputs

## What gets stored

For each artifact, Mnemonic can store:

- artifact hash
- producing agent identity
- input or upstream references
- semantic summary
- timestamp
- associated task or project namespace

## Why this pattern is useful

A2A systems can pass artifacts around, but without attestation it is harder to prove:

- who produced the artifact
- whether it changed later
- what upstream evidence it depended on
- which version was used in a downstream task

Mnemonic gives each artifact a durable provenance trail.

## Example

A verification agent produces a final evidence pack for a journalistic story.

Mnemonic stores:

- the artifact hash
- the source bundle references
- the producing agent identity
- the timestamped attestation

Later, a reviewer can check whether the delivered artifact matches the originally attested version.

## Mnemonic value in this pattern

- tamper-evident artifact history
- retrievable semantic index over produced outputs
- stronger auditability for agent workflows
- durable link between outputs and upstream evidence

## Best fit

Excellent for regulated or high-trust workflows where outputs must be attributable and reviewable.

---

## Reference implementation

**Bridge mode:** sidecar (`bridge-a2a`). The sidecar intercepts `tasks/get` responses and automatically attests every artifact in `task.artifacts[]` under the task's `contextId`.

**Schemas involved:** `A2A_ARTIFACT_V1` (artifact hash, producer identity, parts), `A2A_TASK_V1` (parent task reference via `prev_id` lineage).

```typescript
// Reviewer verifies an artifact by its attestation id.
const row = await client.callTool("mnemonic_find_by_tx", {
  tx_id: "local:att-uuid-1234",
});
const artifact = JSON.parse(row.content);   // JCS-canonical artifact JSON
const hash = row.content_hash;              // blake3 of the JCS bytes
const signer = row.signer_pubkey;           // producer's Ed25519 pubkey (base58)
```

**`recall_by_context` query:** `context_id` matches the parent task's `contextId`. Returns all artifacts and messages produced under that task, allowing a reviewer to reconstruct the full provenance chain.
