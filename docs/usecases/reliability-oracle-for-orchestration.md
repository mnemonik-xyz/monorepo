# Reliability Oracle for Orchestration

## Pattern summary

Use Mnemonic as a reliability oracle that informs A2A orchestration decisions.

Instead of routing work only by stated capabilities, orchestration can also use memory-backed trust signals.

## What gets tracked

Possible reliability inputs include:

- accepted vs rejected outputs
- downstream reuse of contributions
- citation quality history
- contradiction rate
- reviewer corrections
- domain-specific success history

## Why this pattern is useful

Large agent systems need more than interoperability. They need good routing decisions.

An orchestrator should be able to ask:

- which agent is most reliable on this task type?
- whose outputs are regularly reused?
- which contributors should be down-weighted?

Mnemonic can hold the historical evidence needed to answer those questions.

## Example

A coordinator agent must choose between several specialist agents for a legal-analysis task.

Mnemonic provides historical signals showing that one agent:

- performs well on legal retrieval
- has low correction rates
- produces outputs that downstream agents reuse more often

That signal influences task routing.

## Mnemonic value in this pattern

- persistent trust history
- evidence-backed reliability scoring
- better orchestration decisions over time
- compatibility with future per-writer reputation systems

## Best fit

This is a strong later-stage pattern once enough multi-agent interaction data exists. It is especially aligned with Mnemonic's existing interest in reliability scoring and adversarial mitigation.
