---
created: 2026-05-02
status: draft
type: feature
size: M
priority: P1 (post-launch, fixes the recurring footgun behind today's auth debugging)
related:
  - work/cursor-vscode-e2e-tests/manual-verify.md (section G — Send-to-CLI alignment)
  - work/mnemonic-cli/backlog.md (Phase 2 — Crypto-flexibility / Turnkey)
---

# User Spec: Keypair Sync — eliminate localStorage ↔ identity.json drift

## Что делаем

Make the user's Ed25519 keypair behave like a SINGLE THING that lives across CLI + webapp + IDE-pasted-JWT contexts, with explicit operations to mutate it and bidirectional sync to align it. No more silent drift; no more "pending bundle owner mismatch" 403 when the user does `mnemonic init --force` or clicks webapp "Generate new" out of band.

Three-tier delivery:

**Tier 1 — Same-week (~3 dev-days):**

1. **JWT-baked install deeplinks.** Webapp's "Install in Cursor / VS Code" buttons include `headers: { Authorization: "Bearer <current-webapp-JWT>" }` in the install config when the user is logged in to the webapp. Cursor stores it; sends on every `/mcp` call; JWT.sub equals the webapp's localStorage pubkey, which is also the keypair that signs `/api/sign-callback` → no drift possible.
2. **Drift-warning prompts.**
   - Webapp "Generate new": modal "This will replace your current keypair. JWTs your IDEs / CLI hold for the current pubkey will stop working. Options: (a) Download backup JSON first, (b) Send to CLI first, (c) Cancel, (d) Generate anyway."
   - CLI `mnemonic init --force`: already warns; extend the warning to also remind about webapp localStorage drift.
3. **`mnemonic identity status` command.** Reports `(local pubkey, webapp pubkey if known via cached JWT, status: synced | diverged | webapp-unknown)`.

**Tier 2 — Within 2 weeks (~5 dev-days):**

4. **Pull-from-CLI flow** (mirror of Send-to-CLI). New CLI command `mnemonic identity push-to-webapp` issues a ticket via `POST /api/cli-bootstrap/issue-from-cli` (new endpoint). User opens `https://mnemonik.xyz/install?pull=<ticket>` (or scans the QR the CLI prints) — webapp redeems and pre-fills localStorage. Sync is now one-click in either direction.
5. **Auto-detect drift on webapp visit.** When a logged-in webapp session loads `/install`, fetch the current CLI pubkey via a small read-only endpoint (if CLI has registered a "I exist" beacon). On drift: surface a banner "Your CLI uses pubkey X; this browser uses Y. Sync now? [Pull from CLI / Push to CLI / Ignore]".

**Tier 3 — Phase 2 architectural (post-launch, ~10-12 dev-days, alternative path):**

6. **Custodial keypair via Turnkey.** Already in `work/mnemonic-cli/backlog.md` as Phase 2 / TurnkeySigner. Single source of truth, eliminates drift entirely because there's only one store.
7. **Server-side multi-key linkage.** Each user has a primary pubkey + linked-pubkeys table. Sign-callback accepts any linked pubkey for a given primary. CLI and webapp can legitimately use different keypairs, both anchor under the same user identity.

## Зачем

Today's session hit the drift bug at least four distinct ways:

- Cursor 0.1.5 sign — `mnemonic init` created a CLI keypair, `mnemonic login` minted a JWT signed by webapp's localStorage keypair → mismatch → bug #27.
- IDE OAuth — Cursor's MCP UI doesn't surface the OAuth flow → user pasted a CLI JWT manually → JWT.sub = CLI pubkey, webapp signs with localStorage = different pubkey → "pending bundle owner mismatch".
- Webapp test fixtures — `oauth-flow.spec.ts` always generates a fresh keypair on `/install`, so any local CLI identity is irrelevant to the spec; correct for tests but exposes the structural issue.
- Storage flip rollback — restarting the server cleared in-memory OAuth pending state but not JWTs; users with JWTs from the old in-memory state then drifted vs. the new state.

Without a sync story, every release post-mortem will contain a section about keypair drift. With this feature, it becomes a non-issue: the protocol is the same; what changes is that the user EXPERIENCES one keypair across all surfaces.

## Out of scope

- Multi-tenant keypairs (a single user with one CLI + multiple browsers would need a different design — out of scope until we have the use case).
- Migrating existing `local:` synthetic-tx attestations to a new keypair (impossible without re-signing — those rows stay tied to whatever pubkey signed them at the time).
- Hardware-wallet-style multi-device approval. That's WebAuthnSigner / Turnkey territory (Tier 3).

## Acceptance criteria

- [ ] User can install the Mnemonic MCP connector in Cursor / VS Code via the webapp install button and have working tool calls without manually editing `mcp.json` or running `mnemonic login`.
- [ ] If user clicks "Generate new" in the webapp, they see an unambiguous warning AND have a one-click "Send to CLI" / "Download backup" path BEFORE the new key is created.
- [ ] `mnemonic identity status` returns clean output identifying drift if any.
- [ ] One click in either webapp or CLI reverses any drift back to alignment.
- [ ] No new failing path adds to the existing "pending bundle owner mismatch" failure mode.
