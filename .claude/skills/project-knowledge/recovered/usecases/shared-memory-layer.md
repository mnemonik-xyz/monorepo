# Shared Memory Layer

## What it is

Mnemonic can serve as a shared persistent memory layer for A2A agents working on the same task, project, or namespace.

A2A handles agent communication and task exchange. Mnemonic adds a common memory substrate that survives across sessions, processes, providers, and runtime changes.

## Why it matters

Without a shared memory layer, multi-agent systems usually rely on:

- transient context windows
- ad hoc databases
- framework-specific memory implementations
- centralized vendor-controlled storage

These approaches are fragile, hard to audit, and usually not portable.

Mnemonic improves this by providing:

- persistent memory across sessions
- semantic retrieval over shared records
- verifiable provenance of stored items
- portability across agent runtimes and providers
- optional decentralization and non-censorship properties

## Example scenario

A research workflow has four A2A-connected agents:

- source discovery agent
- analysis agent
- writing agent
- verification agent

Each agent contributes findings into a shared Mnemonic namespace:

- discovered sources
- extracted claims
- hypotheses
- contradictions
- writing notes
- reviewer feedback

The next time any agent joins the workflow, it can retrieve the accumulated context instead of starting from zero.

## Why this is a strong fit for Mnemonic

This is the cleanest near-term role because it directly matches the core product thesis:

- agents need shared context
- that context should outlive any one runtime
- memory should not be locked to one vendor or framework

## What Mnemonic contributes

- durable memory store
- semantic recall across agent contributions
- snapshot and restore mechanics
- verifiable history of what was stored and when

## What A2A still contributes

- discovery of other agents
- task routing
- message exchange
- artifact passing
- workflow orchestration

## Recommended priority

Highest priority. This is the most straightforward and defensible A2A building block for Mnemonic.
