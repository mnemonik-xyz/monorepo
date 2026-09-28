# Task Memory Ledger

## Pattern summary

Use Mnemonic as the persistent ledger for A2A task execution history.

Each task exchanged in an A2A workflow leaves behind a durable record that can be retrieved later.

## What gets stored

For each task, Mnemonic can record:

- task request hash
- assigned agent identity
- task summary
- intermediate notes
- output summary
- artifact references
- completion status
- timestamps and ordering anchors

## Why this pattern is useful

A2A workflows often involve many short-lived tasks. Without durable task memory, agents repeatedly lose context such as:

- what has already been tried
- what assumptions were made
- why a task was delegated
- what the result was

A task memory ledger preserves that history in a retrievable form.

## Example

A purchasing workflow has:

- planning agent
- vendor lookup agent
- risk agent
- approval agent

Each delegation becomes a memory entry. Later agents can query:

- which vendors were already rejected?
- what risks were already identified?
- which approval conditions failed?

## Mnemonic value in this pattern

- persistent task history
- semantic recall across previous tasks
- auditable sequence of task execution
- reduced redundant work in long-running agent workflows

## Best fit

Strong for enterprise workflows, research pipelines, operational assistants, and compliance-sensitive agent systems.

---

## Reference implementation

**Bridge mode:** sidecar (`bridge-a2a`). Drop the sidecar in front of any A2A-compliant agent; it intercepts every `tasks/get` and `message/send` call and attests the task without modifying agent code.

**Schemas involved:** `A2A_TASK_V1` (task status + history), `A2A_MESSAGE_V1` (per-turn messages in the task history).

```typescript
// Query the task memory ledger for a context: retrieve the last 20 attestations.
const rows = await client.callTool("mnemonic_recall_by_context", {
  context_id: "ctx-purchasing-workflow-001",
  limit: 20,
});
// Each row: { attestation_id, content (JCS-canonical Task JSON), content_hash }
const lastTask = JSON.parse(rows[0].content);
console.log("last known status:", lastTask.status);
```

**`recall_by_context` query:** `context_id` is the A2A `contextId` field. The sidecar writes one attestation per terminal task event; recall returns them newest-first so the most recent status is always `rows[0]`.
