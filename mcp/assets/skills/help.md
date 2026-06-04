# mnemonik-help

## Purpose

Orient a newly-connected agent (or its operator) to what Mnemonic offers, when each tool applies, and how local-mode and participate-mode differ. Use when the user asks what Mnemonic is, what it can do, or how it works.

## Trigger

**Positive examples (DO use):**

- The user asks "what is mnemonik", "what can you do with memory", "what tools do you have for remembering things", or similar discovery questions.
- The user just connected the MCP server and the agent has no prior context for what to do with it.
- The user references mnemonik by name for the first time and the conversation has no anchor for what the protocol is.
- Another agent in a multi-agent setup asks for capability discovery.

**Negative examples (DO NOT use):**

- The user is asking for the result of a specific operation (recall, attest, verify) — call that tool directly, do not detour through help.
- The user has already received help in this session and is now executing on it — repeating the orientation is noise.
- The user is debugging an error — surface the error and `repair_hint`, do not pivot to general help.

## Context to gather

- Has the user used Mnemonic before in this session? If yes, scope help to what they have not used yet rather than restating everything.
- Is the user on a personal machine (`mcp-stdio` subprocess) or hitting the hosted endpoint anonymously? Local-mode flows only apply to the former.

## Tool

No MCP tool call. This skill is purely informational: surface the list of available skills (`mnemonik-init`, `mnemonik-recall`, `mnemonik-attest`, `mnemonik-checkpoint`, `mnemonik-verify`, `mnemonik-status`) with one-line purposes, and the two-mode mental model (local = offline, free, private; participate = chain-anchored, paid, optionally public).

## Guardrails

- Do not enumerate every tool argument in the help response — that is the per-skill manifest's job. Help is a map, not the territory.
- Do not invent capabilities that are not in the manifest list. If the user asks about something not covered (e.g., "can it share with other users"), be direct that v1 does not include that.
- Do not promise behavior that depends on configuration the agent has not verified (e.g., do not claim participate-mode works before the user has connected their identity).
