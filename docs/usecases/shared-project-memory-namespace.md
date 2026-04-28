# Shared Project Memory Namespace

## Pattern summary

Use Mnemonic as a shared project-level memory namespace that multiple A2A agents can read from and write to.

Instead of each agent carrying isolated memory, the project itself has a durable memory surface.

## What gets stored

A project namespace can contain:

- findings
- notes
- unresolved questions
- source references
- artifacts
- decisions
- contradictions
- confidence annotations

## Why this pattern is useful

This is the most direct pattern for multi-agent collaboration.

When multiple agents work on the same long-running effort, they need a common memory surface so that:

- context accumulates over time
- agents do not duplicate work
- handoffs between specialists are smoother
- switching models or providers does not break continuity

## Example

A research project has four agents:

- source discovery
- summarization
- synthesis
- verification

Each agent adds to the same namespace. The writing agent can later query all prior discoveries without depending on the runtime that originally produced them.

## Mnemonic value in this pattern

- durable shared context
- semantic retrieval over project history
- portable memory independent of a single framework
- foundation for future multi-party reliability scoring

## Best fit

This is likely the highest-priority integration pattern for Mnemonic in A2A systems.
