# 0010. Operator and agent registries on ERC-8004, starting on Arc

- Status: Accepted, details open
- Date: 2026-10-10

## Context

For anyone to run an operator, clients need to find operators, check them and hold them
to account. Agents that negotiate with each other need to find and verify each other.

## Decision

- Use ERC-8004 identity, reputation and validation registries, starting on Arc.
- **Operator registry:** endpoint, operator key, capabilities, prices, stake. Listing needs
  a stake, proven uptime and declared capabilities.
- **Agent registry:** agent key, agent card (A2A), operators, owner. Registration is opt-in
  for agents that want to be found. Entries are public. Agents stake a small amount.
  One user can own any number of agents.
- The SDK ranks operators by availability first.
- Users pin trusted operators; the registry suggests and never replaces a pin.
- Operators find and talk to each other through the registry.
- Operators stake a protocol token.
- A separate judges role receives proofs of signature-provable faults and decides
  slashing. Other problems go into reputation.

## Consequences

- A protocol token needs design and legal review before any offering; a DAO is the
  usual governance model.
- Open: minimum stake (about 1000 USD equivalent proposed) and unbonding period;
  where slashed funds go (harmed users first, then a DAO treasury, is common practice);
  the list of slashable faults; token supply, distribution and rewards.

## Alternatives considered

- A custom registry contract: more work, less interoperable.
- Stake in USDC only: simpler, but no protocol token economics.
