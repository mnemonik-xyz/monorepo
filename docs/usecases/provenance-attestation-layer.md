# Provenance and Attestation Layer

## What it is

Mnemonic can act as a provenance and attestation layer for A2A workflows.

In this role, Mnemonic is not just storing memory. It is recording what an agent produced, what inputs it used, when it produced the output, and how that output connects to earlier artifacts.

## Why it matters

A2A can move artifacts between agents, but it does not by itself guarantee:

- who created an artifact
- whether it was altered later
- what upstream task or memory produced it
- whether a claim existed at a specific time

For serious multi-agent workflows, provenance matters as much as coordination.

## Example scenario

An A2A workflow produces:

- a report draft
- an evidence bundle
- a decision summary
- a compliance explanation

Mnemonic stores attestations for:

- task request hash
- artifact hash
- producing agent identity
- timestamp and ordering anchor
- links to upstream source material
- semantic summary for retrieval

This makes it possible to later prove:

- who produced the artifact
- what the artifact version was
- which evidence chain led to it

## Useful domains

This role is especially valuable in:

- investigative journalism
- legal workflows
- compliance and audit
- scientific collaboration
- enterprise decision pipelines

## What Mnemonic contributes

- tamper-evident storage of artifact metadata
- durable references between artifacts and memories
- cryptographic identity linkage
- auditable timeline of knowledge and outputs

## What A2A still contributes

- orchestrating which agent does the work
- passing tasks and artifacts between agents
- handling runtime coordination and status updates

## Why this role is strong

This is one of the strongest ways to make Mnemonic materially useful inside multi-agent systems. It turns agent workflows from opaque message passing into auditable knowledge production systems.

---

## Reference implementation

**Bridge mode:** sidecar (`bridge-a2a`) with `FAILURE_MODE=attest-strict`. The sidecar fails the proxied response if attestation fails, ensuring no un-attested artifact reaches downstream agents.

**Schemas involved:** `A2A_ARTIFACT_V1` (artifact hash + producing agent pubkey), `A2A_TASK_V1` (parent task with `prev_id` lineage linking artifact to task to message chain).

```typescript
// Attest an artifact with explicit upstream lineage.
await client.callTool("mnemonic_attest_a2a", {
  object_type: "artifact",
  context_id: taskContextId,
  prev_id: parentTaskAttestationId,   // lineage: artifact → task → messages
  payload: JSON.stringify({
    artifactId: "art-evidence-001", name: "evidence-bundle",
    parts: [{ kind: "data", data: evidenceJson, mimeType: "application/json" }],
  }),
});
```

**`recall_by_context` query:** `context_id: taskContextId` returns all attestations in the workflow. Walk `prev_id` links to reconstruct the causal chain from initial message through task to final artifact.
