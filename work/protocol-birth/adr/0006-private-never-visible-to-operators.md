# 0006. Private memories are never visible to operators

- Status: Accepted, not yet implemented
- Date: 2026-10-10

## Context

When a web agent such as Claude on claude.ai saves a memory, it calls the connector tool
with the text as an argument. Anthropic's servers send that call to the operator's URL,
so the operator receives the text before any browser takes part. The browser can
encrypt and sign in WebAssembly, but too late on this path.

## Decision

- An operator never sees the plaintext of a private memory.
- Web agents can write private memories. The agent gives the user a link such as
  `https://www.mnemonik.xyz/save#<text>`. The part after `#` never reaches a server.
  The page encrypts and signs with the user's key in WebAssembly and uploads to Turbo.
- Local MCP, SDK and extension clients encrypt and sign locally.
- Public memories keep the hosted path.

## Consequences

- One click per private save from a web agent.
- Very long memories make long links (browsers accept fragments of hundreds of KB).
- Anthropic still sees text that is part of a Claude conversation.
- Open: how the agent learns the link format without a tool call that carries the text.

## Alternatives considered

- Operator in a hardware enclave: strong, but complex hosting and attestation.
- The agent encrypts with a code tool: rejected; agents do not run code (0012).
- Operator sees the text briefly: breaks the rule.
