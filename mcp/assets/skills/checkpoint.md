# mnemonik-checkpoint

## Purpose

Capture a project-state snapshot — what was done, what is next, what is blocked — as a single attestation that another agent (or the user, on a different day) can recall to resume context. Use at natural session boundaries: end of a coding session, end of a research thread, before handing off to a collaborator.

## Trigger

**Positive examples (DO use):**

- The user says "let's wrap up", "end of day", "let's stop here", "save where we are".
- The agent is about to switch to an unrelated task and a context-bridge would help the next session resume.
- A long debugging or research thread reaches a natural pause (not a conclusion — the conclusion would be `mnemonik-attest`).
- The user is handing off to a teammate and explicitly asks to save state.

**Negative examples (DO NOT use):**

- The user is still actively working — checkpoint is for boundaries, not continuous progress.
- The session was short or trivial (a one-liner question, a quick lookup) — checkpoint has overhead.
- A consequential decision was made — that is `mnemonik-attest`, not checkpoint. Checkpoints summarize state; attestations capture decisions.
- The user said "remember this specific fact" — that is `mnemonik-attest` on the fact, not a state summary.

## Context to gather

- **What was accomplished** in this session — concrete outcomes, not narration.
- **What is next** — the immediate-next action when the user (or another agent) picks this up.
- **What is blocked** or waiting on external input.
- **Scope tags** — project name, module, branch — so future recall can scope to this thread.

## Tool

Underlying MCP tool: `mnemonic_sign_memory` (checkpoints are a kind of attestation).

Arguments:

- `content` (string, required) — three sections: Done, Next, Blocked. Keep each tight.
- `tags` (array of strings, optional) — include `["checkpoint", "<project>", "<branch>"]` so recall can filter.
- `mode` (`"local"` | `"participate"`, optional) — default `local`. Checkpoints are usually private working state, so participate is almost never appropriate.
- `visibility` — do not set. Checkpoints are personal state.
- `allow_fallback_to_participate` — leave default `false`. A failed local checkpoint should surface as an error, not silently publish.

## Guardrails

- Default to `mode="local"`. Checkpoints capture working state that is rarely meant for the public pool.
- Do not include credentials, session tokens, or PII from the working context. Strip them before forming the content string.
- Do not write checkpoints on every turn. One per session boundary is the cadence.
- If the user has multiple parallel projects, write one checkpoint per project rather than a single mega-checkpoint that mixes scopes — recall will return better results.
