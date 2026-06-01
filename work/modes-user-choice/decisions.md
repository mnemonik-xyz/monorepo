# Decisions — modes-user-choice

Append-only log of decisions and audit findings.

## Open decisions (awaiting user sign-off before Wave 1)

1. **Storage invariant** — S1 (tag rows in one DB, recommended) vs S2 (separate
   DB per mode). User answered "decide in the spec"; tech-spec recommends S1.
2. **Delivery definition for V1** — D1 (durable-anchor + read-back, recommended)
   vs D2 (recipient ack). tech-spec recommends D1, schema forward-shaped for D2.
3. **Mode granularity** — per-request `mode` field on `sign_memory`
   (recommended) vs per-identity default vs both.

## Locked context (from user, 2026-06-01)

- Local storage (filesystem / self-hosted) ⇒ **free, no payment**. Structural,
  per whitepaper §5.7.1.
- "Participate" (share/exchange artifacts with the network) ⇒ protocol must
  **guarantee delivery**; purely-local artifacts are risky to share because they
  may be unreachable when a peer asks. This is the paid service-layer path.
- Mode must be the **user's** choice driven by intent, not an operator env var.
