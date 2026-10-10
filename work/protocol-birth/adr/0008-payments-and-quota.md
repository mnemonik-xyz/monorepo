# 0008. Payments: free tier, x402 to the operator, Turbo credit sharing

- Status: Accepted, details open
- Date: 2026-10-10

## Context

With author signing (0005), Turbo charges the account that pays, not the signer. Turbo
supports credit sharing: a paying account approves an amount for another address, and
uploads that send `x-paid-by: <payer>` use it. A 16 KiB memory costs about $0.0014 from
Turbo (2026-10-10).

## Decision

| Tier | Who pays | Mechanism |
| --- | --- | --- |
| Free | Nobody | Turbo free tier per user wallet |
| Operator-funded | User pays the operator per write | x402 (USDC); the operator grants a Turbo credit share approval; uploads send `x-paid-by` |
| Self-funded | User | Turbo credits on the user's own address |

The operator sizes per-user quotas from usage statistics. Relay of agent-to-agent
messages is paid by the agent or user.

## Consequences

- The operator's business includes selling writes and managing credit approvals.
- A margin of 1% over Turbo cost would not fund an operator; pricing needs an economic study.
- Open: price per write, quota formula, how an x402 payment maps to a credit approval,
  behavior behind shared IP addresses, protocol fee size.

## Alternatives considered

- User always pays Turbo directly: every user must fund an account first.
- Operator pays everything without charging: not sustainable.
