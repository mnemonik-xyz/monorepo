# mnemonik-status

## Purpose

Report on the local Mnemonic installation's health: which host configs contain a mnemonik entry, whether the hosted endpoint is reachable, binary integrity, local SQLite read/write, identity availability, and token availability. Use when the user reports something is broken, or when the agent needs to know which capabilities are currently available before suggesting a workflow.

## Trigger

**Positive examples (DO use):**

- The user reports an error and the agent does not know which subsystem failed — status gives a triage map.
- The user just finished installing and wants to confirm everything is wired correctly.
- The agent is about to suggest a participate-mode flow and wants to confirm the token / OAuth path is healthy first.
- The user asks "is this working", "is mnemonik set up", "what's the state of my install".

**Negative examples (DO NOT use):**

- The user has explicitly described the problem already (e.g., "my SQLite is locked") — go directly to the fix, do not run status to rediscover it.
- The user is in the middle of a working operation — status interrupts the flow.
- Every connection or every tool call — status is for diagnosis, not a health check on the happy path.

## Context to gather

- Whether a specific subsystem is suspected (host config, hosted reachability, binary integrity, SQLite, identity, token).
- Whether the user has performed any participate writes — the token check is informational on a fresh install, load-bearing once OAuth has been used.

## Tool

Underlying CLI command: `mnemonik-mcp doctor` (run by the shim, not via MCP `tools/call`). It performs structured pass/fail per check with `repair_hint` per failure. Exit 0 on all-pass, exit 1 on any fail.

## Guardrails

- Do not paraphrase the `repair_hint` — surface it verbatim. The hint is engineered to be actionable.
- Do not suggest fixes that contradict the doctor output. If doctor says the host config is fine, do not propose re-running install.
- Do not run doctor in a loop. One run, surface the result, then act on the worst failure.
- If `doctor` itself fails to start, surface that as the binary integrity check failing — do not pretend other checks ran.
