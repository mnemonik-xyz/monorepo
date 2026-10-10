# 0011. Identity version 1: one key per user

- Status: Accepted, not yet implemented
- Date: 2026-10-10

## Context

In the live test, signing failed twice with "owner mismatch": the account was tied to one
key, and the browser used for approval held a different key. Device keys, revocation and
passkey backups would solve this, but add a lot of protocol work.

## Decision

Keep one key per user and add three client features:

1. **Recovery phrase, optional and encouraged.** Shown once at key creation with a Skip
   button; reminders at key moments; never blocking; can be shown later from a device
   that has the key.
2. **One-step key transfer** to another browser or device by QR code or link, encrypted
   in transit.
3. **Private memories encrypted to the user's key,** so every device with the key reads them.

When no key is present, the page offers: restore the key, create a new key (first-time
users only), or cancel (nothing is saved, and the agent is told the user declined).

Device keys, a registry device list, on-chain revocation and passkey backup come later,
only when needed.

## Consequences

- No protocol change: a memory is still signed by one key.
- A lost device loses nothing that was uploaded: memories live on Arweave.
- Accepted risk: a stolen, unlocked device exposes the one key. The response is a new
  key and moving access to it by hand.
- Without a recovery phrase, losing every device loses the identity and its private
  memories for good.

## Alternatives considered

- Device keys under one identity, with passkey backup: decided first, then deferred as
  too complex for version 1.
- Rebind the account to whatever key a browser has: creates a different identity.
