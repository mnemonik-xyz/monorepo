# mnemonik-recall

## Purpose

Search the user's attested memory pool (or the public pool, if anonymous) by semantic similarity and return matching attestations ranked by relevance. Use when the user (or the current task) needs prior context that a stored memory likely captured.

## Trigger

**Positive examples (DO use):**

- The user asks "what did I decide about X", "why did we pick Y", "have I worked on Z before".
- The current task starts and the agent needs prior context — recall on the project name or module to pull what was previously attested.
- The user references a past decision vaguely ("the thing we figured out about auth last month") — recall on the topic.
- Anonymous discovery: the user wants to see public attestations on a topic and the agent has no auth — anonymous recall against the hosted endpoint returns only `visibility=public` rows.

**Negative examples (DO NOT use):**

- The user is asking for live data (current weather, current git log, current PR list) that an attestation cannot provide — use the appropriate live tool.
- The user is asking for general knowledge unrelated to their own attested memory — answer from your own knowledge or use a search tool.
- The recall would be on an empty query, a single word with no semantic content, or otherwise too thin to return useful results.

## Context to gather

- The **query** in plain language — the recall embedder semantically matches, so natural phrasing works better than keyword soup.
- The **limit** — how many results the user wants surfaced. Default 5 is fine for most cases; raise for exploration, lower for "best match only."
- Whether the caller is authenticated. Anonymous callers see only `visibility=public` rows; authenticated callers see all their own rows regardless of visibility.

## Tool

Underlying MCP tool: `mnemonic_recall`.

Arguments:

- `query` (string, required) — natural-language query string.
- `limit` (integer, optional, default 5) — max number of results to return.

Returns a list of attestations with content, tags, score, attestation_id, write_mode, and timestamps.

## Guardrails

- Recall against the **local** SQLite returns the user's own attestations regardless of visibility — local writes are implicitly private and never leave the machine.
- Recall against the **hosted** endpoint without a JWT filters by `visibility=public` only. Do not promise the user that anonymous recall surfaces their own private memories — it does not.
- Cross-mode recall is NOT guaranteed in v1. A local write is searchable in local recall; a participate write is searchable in hosted recall. They do not bridge.
- Do not interpret a low-score result as proof of absence — the embedder may simply not have a close enough match. If the top-1 score is under ~0.4, surface "no strong match" rather than the row.
- If `embedder.model_version` mismatch was logged at startup (stderr line), warn the user that cross-mode recall consistency is undefined.
