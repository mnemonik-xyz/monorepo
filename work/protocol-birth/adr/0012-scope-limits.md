# 0012. Agents do not run code; escrow belongs to Warrant

- Status: Accepted
- Date: 2026-10-10

## Context

Two proposals stretched Mnemonic past its purpose: letting the agent encrypt data with a
code tool before saving, and adding escrow for paid agent-to-agent deals.

## Decision

- Agents (for example Claude) do not run code to protect or prepare Mnemonic data.
  Encryption and signing happen only in user-controlled clients.
- Escrow and payment guarantees between agents are out of scope for Mnemonic. They belong
  to the separate **Warrant** protocol. Mnemonic records signed memories and messages;
  agent-to-agent payments in Mnemonic stay plain pay-per-call (x402).

## Consequences

- The web-agent private path is the browser link only (0006).
- Integration with Warrant, where needed, is a separate design.

## Alternatives considered

- Agent-side encryption with a code tool: depends on the agent having code execution and
  running it correctly.
- Escrow inside Mnemonic: duplicates Warrant.
