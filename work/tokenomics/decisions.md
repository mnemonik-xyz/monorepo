# Decisions log: tokenomics

Append-only. Owner decisions, task reports and audit findings go here.

---

## Fixed inputs (owner)

- **2026-09-28.** Design the protocol economics: where a native token can circulate, and a
  mechanism that funds development. Be critical and follow best practice.
- **2026-09-28.** The protocol is becoming chain-agnostic and storage-agnostic
  (`work/DECOUPLING-SEQUENCE.md`). The economics must not re-couple it to one chain.

## Audit findings (2026-09-28)

- **F1. One paid action.** Only `mnemonic_sign_memory` with `mode: "anchored"` is paid
  (`CLAUDE.md` § Payment). Price is cost plus 20 % margin, floor 0.001 USD
  (`mcp/src/pricing.rs:7`, `mcp/src/config.rs:329`, `docs/WHITEPAPER.md` § 13).
- **F2. Pay after delivery exists.** An anchored write is charged only after its bytes pass a
  recall and verify round trip. A relayer therefore needs no bond.
- **F3. The Merkle commitment exists as a library.** `core/src/merkle.rs` builds a per-owner root
  and inclusion proofs. Anchoring the root is not built. It is the basis for provable indexer
  omission.
- **F4. Cost is falling.** Memo removal (`work/chain-agnostic/`) and batching lower the fee per
  write. Fee-based token value falls with it.
- **F5. Docs mismatch, outside this spec.** `docs/WHITEPAPER.md` § 5 and § 13 say "100 free per
  week per identity (planned)". The code ships 10 per Google account per day
  (`economics.md`, `docs/tools.md`).

---

## Decisions (proposed, awaiting the owner)

### D-1. No token before the gates (proposed 2026-09-28)

Build the stablecoin fee architecture now (Phase 0 and 1). Launch `MNEM` only after gates G-1 to
G-6 of `tech-spec.md` § 8 hold for six months. Reason: the fee base cannot support a token, and
no current role needs one (tech-spec § 1, § 4).

### D-2. Users pay in stablecoins, for ever (proposed 2026-09-28)

Invariant I-3. The token is never a payment currency for writes, reads, restore or verify.

### D-3. Security in USDC, alignment in the token (proposed 2026-09-28)

Slashable bonds are USDC. `MNEM` stake decides reward share and routing weight only. Reason:
a native-only stake is reflexive (tech-spec § 5).

### D-4. Buyback pays workers; it does not burn (proposed 2026-09-28)

Bought-back `MNEM` pays for verified work. Counsel must confirm this choice (tech-spec § 7.2).

### D-5. Development funding comes from revenue first (proposed 2026-09-28)

The development share (M-1) starts in Phase 0, in USDC. Token allocations are a supplement,
never the salary base.

## Open questions (owner)

- **Q-1.** Accept D-1, "no token before the gates"?
- **Q-2.** The router split. Proposal: Phase 0–1 = 60 % development, 25 % network programs,
  15 % reserve. Phase 2 = 40 % development, 40 % buyback, 20 % reserve.
- **Q-3.** Legal wrapper and treasury signers: a foundation, a company, or both? Who holds the
  multi-signature keys?
- **Q-4.** Home chain for a future token. Criteria are in tech-spec § 7.7. The ERC-8004 work
  points to EVM; Ed25519 fault proofs point to a chain where that check is cheap.
- **Q-5.** Governance: token vote, contributor council, or both (tech-spec § 7.6)?
- **Q-6.** The numbers for gates G-1 to G-3.
- **Q-7.** Raise outside money or not? It changes the 7.4 allocation and the legal work.
