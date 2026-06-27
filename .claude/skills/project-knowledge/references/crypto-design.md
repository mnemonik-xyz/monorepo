# Crypto Flexibility — Architectural Notes

Status: **draft notes for future deliberation**. Captures the cost picture of decoupling the protocol from Solana Ed25519 lock-in, the two-signer architecture, and the touchpoints that need to change. Not a current-state document — current state is single-signer Ed25519. This is the migration path.

---

## Current state (Phase 1, 2026-04)

Identity is hard-pinned to Solana Ed25519 across every layer:

| Layer | Lock-in |
|---|---|
| `core/src/identity/mod.rs` | `generate_keypair` returns Ed25519 only; DID format always `did:sol:<base58>` |
| `core/src/codec/sign.rs` | COSE_Sign1 alg field hardcoded to `-8` (Ed25519/EdDSA) |
| `core/src/storage/sqlite.rs` | `signer_pubkey TEXT` column; no `signer_alg` companion |
| `core/src/arweave/` | ANS-104 bundle item signing uses Ed25519 |
| `core/src/solana/` | SPL Memo writer requires Solana keypair (SVM is Ed25519-only) |
| `core/src/wasm/mod.rs` | `sign_challenge` / `sign_cose_payload` / `generate_keypair` accept only Ed25519 keypair JSON |
| `mcp/src/oauth.rs` | Verify routes assume base58 Ed25519 pubkey for `sub` claim |
| Webapp `localStorage["mnemonic.identity"]` | `{secret: number[64], pubkey_base58}` — Solana keypair shape verbatim |
| Server `MNEMONIC_KEYPAIR_PATH` | Ed25519 file format |

**Identity == Anchor ID.** Same keypair signs the off-chain attestation envelope AND the on-chain Solana Memo. Must be Ed25519 because Solana SVM requires it.

**Signer interface in SDK is already abstract** (Phase 1 design choice — `interface Signer { pubkey: string; sign(bytes): Promise<bytes> }`). This is the only door currently open for swap-in. Server side and storage are not abstracted.

---

## Why decoupling matters

Real users locked out today:

- **WebAuthn / passkeys** — Touch ID, Yubikey, Windows Hello, Android passkey. Use ES256 (secp256r1) or RS256, never Ed25519. User cannot sign attestations from their browser passkey today.
- **Hardware wallets** — Ledger, Trezor: secp256k1 by default, occasionally Ed25519 via app. The default is wrong.
- **HSMs / cloud KMS** — AWS KMS, GCP KMS, Cloudflare KMS support various algs but not all support Ed25519. Locks out enterprise integrations.
- **Corporate identity** — SAML / OAuth corporate identities are typically RSA-2048 or ECDSA-P256. Single-sign-on flows can't issue an Ed25519-signed attestation.
- **Post-quantum migration** — ML-DSA (Dilithium), SLH-DSA (SPHINCS+), Falcon. Pinning to Ed25519 forecloses the post-quantum upgrade.

---

## Target architecture: two-signer split

```
Identity
├── Off-chain envelope signer (alg-pluggable)
│   - COSE_Sign1 supports any registered alg via the alg field
│   - Ed25519 (current default)
│   - secp256k1 (Bitcoin / Ethereum personal-message style)
│   - ES256 / secp256r1 (WebAuthn passkeys)
│   - RS256 (corporate SAML identities)
│   - future: ML-DSA / Falcon (post-quantum)
└── Anchor signer (chain-pluggable)
    - Solana — Ed25519 (current default; SVM-required)
    - Ethereum — secp256k1 (future, anchors via tx with calldata hash)
    - Bitcoin — Schnorr/ECDSA (future, OP_RETURN anchor)
    - Arweave — alg-agnostic (any Ed25519 ANS-104 keypair, or even none)
    - Local/none — testing
```

Each attestation row records BOTH signers. Verification checks them independently. User picks the combination per-attestation (default: same Ed25519 for both, current behavior).

---

## Cost estimate

### Option B — off-chain pluggable, anchor stays Solana Ed25519

**~6–8 dev-days.** Unblocks 80% of the locked-out use cases (WebAuthn, KMS, corporate identities). Anchor stays Solana so the on-chain story is unchanged.

Touchpoints:

| Component | Change | Effort |
|---|---|---|
| `core/src/identity/mod.rs` | Generic `Signer` trait + Ed25519/secp256k1/ES256 implementations | 1.5 days |
| `core/src/codec/sign.rs` | Set COSE alg field from signer; verify any registered alg | 0.5 day |
| `core/src/storage/sqlite.rs` | Add `signer_alg TEXT NOT NULL DEFAULT 'EdDSA'` column + idempotent migration helper | 0.5 day |
| `core/src/wasm/mod.rs` | Generic sign/verify exports parameterized by alg | 1 day |
| `packages/sdk/src/signer.ts` | Drop-in for new impls — interface already abstract | 0.5 day |
| Webapp `localStorage` | Versioned shape `{alg, secret_*, pubkey_*}` + migration UX | 1.5 days |
| Migration | Legacy NULL `signer_alg` rows → assumed `EdDSA` (Ed25519) | 0.5 day |
| Tests + golden fixtures | Expand fixtures to cover multiple algs | 1 day |
| Docs | 0.5 day |

### Option A — full multi-alg (off-chain + chain-pluggable anchor)

**~10–12 dev-days.** Builds Option B + a chain-anchor adapter pattern.

Adds:

| Component | Change | Effort |
|---|---|---|
| `core/src/anchor/` (new module) | Trait `Anchor` with `write(hash, signer) → tx_id`, `verify(tx_id, expected_hash) → status` | 2 days |
| `core/src/solana/` | Refactor as `Anchor::Solana` impl | 0.5 day |
| `core/src/ethereum/` (new) | `Anchor::Ethereum` — anchors via `eth_sendRawTransaction` calldata hash | 2 days |
| `core/src/arweave/` | Already alg-agnostic — minor refactor to fit `Anchor` trait | 0.5 day |
| `mcp/src/main.rs` | Multi-anchor config (env vars `ANCHOR_BACKEND=solana|ethereum|arweave`) | 0.5 day |

---

## Phase 1 architecture choices that keep the door open

These were intentional Phase 1 designs to avoid foreclosing crypto-flex:

1. **`Signer` interface** in SDK (`packages/sdk/src/signer.ts`) — abstract by design. Future `TurnkeySigner`, `WebAuthnSigner`, `KMSigner` are drop-in. See Decision 4 of `mnemonic-cli` tech-spec.
2. **COSE_Sign1 envelope** — COSE inherently supports any registered alg via the `alg` field. We hardcoded Ed25519 in Phase 1 but the format doesn't require it.
3. **`signer_pubkey TEXT`** in SQLite — a string column that can hold any encoding (base58 today, multibase tomorrow).
4. **Canonical CBOR** is alg-agnostic — `core/src/codec/canonical.rs` doesn't know what crypto is involved.
5. **Tenant scoping by `owner_pubkey`** — uses the JWT `sub` claim, which is just a string. Doesn't care about the alg.

The blockers for Option B are the schema migration + WASM exports + webapp UX, not the architectural skeleton.

---

## Risk: drift from server's Rust CBOR encoder

If pure-JS COSE/CBOR rewrite (`@noble/curves` swap from the bundle-size backlog) lands together with Option B's multi-alg work, **golden fixture coverage must expand to cover every alg × every input shape**. Otherwise a JS encoder bug for, say, ES256 could allow a tampered envelope to validate. This is solvable but doubles the fixture matrix.

Recommendation: keep WASM-first signing (current Phase 1 design) for Option B. Pure-JS swap is independent backlog work.

---

## Recommended sequencing — LOCKED IN (2026-04-30)

| Phase | Scope | Status |
|---|---|---|
| Phase 1 (current) | Single Ed25519 signer, Solana anchor | Shipped via mnemonic-cli. `Signer` interface ready for future swaps. |
| **Phase 1.x — confirmed** | **Turnkey custody (`TurnkeySigner`)** | Drop-in `Signer` impl. Email/passkey recovery, no protocol change. ~3–5 dev-days. Pre-requisite for public launch. |
| Phase 1.5 | On-chain `STORAGE_MODE=full` + billing | Backlog TOP PRIORITY 1. Headline value of "verifiable memory" needs real anchoring before crypto-flex matters publicly. |
| **Phase 2 — confirmed** | **Option B** — off-chain crypto-flex | Unblocks passkey + KMS users. Anchor stays Solana. ~6–8 dev-days. |
| **Phase 3 — confirmed** | **Option A** — chain-pluggable anchor | Frees protocol from SVM dependency. Adds Ethereum/Bitcoin/etc. anchors. ~3–5 dev-days on top of Option B. |

---

## Phase 1.x — Turnkey custody integration (detailed)

**Why it slots between Phase 1 and Phase 1.5:** Phase 1's `LocalSigner` stores raw Ed25519 secret in `localStorage` (browser) or `~/.mnemonic/identity.json` (CLI). If user loses the device or clears storage — keypair gone, attestations unrecoverable. Public launch requires email/passkey recovery; Turnkey is the cleanest path.

**Why it doesn't need full Option B:** Turnkey supports Ed25519 natively. Same alg, same protocol contract — only the `Signer.sign()` path goes through Turnkey API instead of WASM. Server side, COSE envelope, anchor — unchanged.

### Architectural picture

```
SDK
├── Signer (interface, Phase 1)
│   ├── LocalSigner (Phase 1) — secret in memory, signs via WASM sign_challenge
│   └── TurnkeySigner (Phase 1.x) — secret in Turnkey Sub-Org, signs via signRawPayload activity
```

Turnkey supports: **Ed25519** (✓ Phase 1.x), secp256k1 (Phase 3 Ethereum), secp256r1/ES256 (Phase 2 passkeys). Same vendor, multiple algorithms — Phases 2 and 3 can reuse the same Turnkey integration without re-platforming.

### `TurnkeySigner` skeleton

```typescript
import type { Signer } from '@mnemonik-xyz/sdk';
import { TurnkeyClient } from '@turnkey/sdk-server';

export class TurnkeySigner implements Signer {
  constructor(
    private client: TurnkeyClient,
    public readonly pubkey: string,
    private readonly privateKeyId: string,
  ) {}

  async sign(bytes: Uint8Array): Promise<Uint8Array> {
    const { activity } = await this.client.signRawPayload({
      organizationId: this.client.organizationId,
      privateKeyId: this.privateKeyId,
      payload: Buffer.from(bytes).toString('hex'),
      encoding: 'PAYLOAD_ENCODING_HEXADECIMAL',
      hashFunction: 'HASH_FUNCTION_NOT_APPLICABLE',  // Ed25519 has no pre-hash
    });
    const signatureHex = activity.result.signRawPayloadResult.signature;
    return Uint8Array.from(Buffer.from(signatureHex, 'hex'));
  }
}
```

Drop-in for `MnemonicClient`:

```typescript
const client = new MnemonicClient({
  baseUrl: 'https://mcp.mnemonik.xyz',
  signer: new TurnkeySigner(turnkeyClient, pubkey, privateKeyId),
  jwt,  // OAuth JWT, same flow
});
```

### Webapp `localStorage` migration

Current shape (Phase 1):
```json
{ "secret": [n0, n1, …, n63], "pubkey_base58": "…" }
```

Turnkey-aware versioned shape (Phase 1.x):
```json
{
  "version": 2,
  "type": "turnkey",
  "sub_org_id": "…",
  "private_key_id": "…",
  "pubkey_base58": "…"
}
```

Or a `LocalSigner` profile under the same envelope:
```json
{ "version": 2, "type": "local", "secret": [n0, …, n63], "pubkey_base58": "…" }
```

Migration: missing `version` → assumed `type: local` legacy shape. Webapp's `IdentityPanel` switches between local/turnkey based on `type` field.

### Server-side impact

**None.** Server validates COSE_Sign1 envelope with raw Ed25519 signature and base58 pubkey. Doesn't care whether the signature came from a local secret or a Turnkey API call. The OAuth challenge sign flow uses `Signer.sign()` polymorphically — both `LocalSigner` and `TurnkeySigner` satisfy it.

### CLI integration

`mnemonic identity import --turnkey` flow:
1. CLI invokes Turnkey's email-auth or passkey-auth flow (browser-based, similar to OAuth `mnemonic login`).
2. Receives Turnkey API credentials + `sub_org_id`.
3. Creates a Sub-Org with one Ed25519 wallet (or imports existing).
4. Saves `~/.mnemonic/identity.json` with the versioned `{type: 'turnkey', ...}` shape.
5. Subsequent `mnemonic sign` uses `TurnkeySigner` instead of `LocalSigner`. Network call per signature — adds ~200ms latency.

### Cost & operational considerations

- **Turnkey pricing:** ~$0.001 per signing operation at typical tiers. Free tier usually covers hackathon/dev. Production: `mnemonic sign` cost = `STORAGE_MODE=full` cost + Turnkey sig cost. Add to economics.md when Phase 1.x lands.
- **Latency:** ~150–300ms round-trip per `sign()` call. Local sign is <1ms. UX-wise: visible but tolerable.
- **Uptime dependency:** if Turnkey is unavailable, users with Turnkey-managed identities cannot sign. Mitigation: `LocalSigner` remains supported for users who prefer self-custody.
- **Vendor lock-in:** soft — `TurnkeySigner` is one impl of `Signer`. Any alternative MPC provider (Privy, Web3Auth, Lit Protocol) is also drop-in. Architecturally not locked.
- **Recovery UX:** main reason to ship Phase 1.x. Lost device + Turnkey passkey/email recovery → keypair restored → all attestations still verifiable. Without this, public launch has too much "user loses everything" risk.

### Effort

| Item | Days |
|---|---|
| `TurnkeySigner` SDK class + tests + integration with mock Turnkey | 1 |
| Webapp `IdentityPanel` Turnkey enrollment + localStorage versioned shape + migration | 2 |
| CLI `mnemonic identity import --turnkey` flow | 1 |
| Server-side audit (verify nothing assumes secret is local) — should be no-op but worth a sweep | 0.5 |
| Docs + README example update | 0.5 |
| **Total** | **~3–5 dev-days** |

### Open decisions before Phase 1.x

- **Sub-Org per user vs shared Org with read-only sub-orgs?** Sub-Org-per-user is industry standard for end-user MPC; allow user to revoke / export.
- **Authentication chain:** Mnemonic OAuth → Turnkey passkey, or Turnkey passkey → Mnemonic OAuth, or both side-by-side? Recommend: OAuth issues Mnemonic JWT, Turnkey passkey is the actual signing authority. Two factors, complementary.
- **Pricing pass-through:** does Mnemonic absorb the Turnkey sig cost or surface to user? Tied to Phase 1.5 billing decisions.
- **Self-custody escape hatch:** can a user export their Turnkey-managed key to a `LocalSigner` profile? Turnkey allows export — UX flow needed.

---

## Verifiable trajectories — STEP/VERDICT/TRAJECTORY envelopes

(`trajectory-experimental`; full design in `work/verifiable-trajectories/`.)

Three new artifact schemas reuse the **unchanged** envelope (deterministic CBOR
→ blake3 → COSE_Sign1 / Ed25519) — no new crypto:

- **`STEP_V1`** — one ordered step. `seq`, `trajectory_id`, and `prev_hash` are
  inside the signed payload; `prev_hash[i] == content_hash[i-1]`, so the chain
  link is tamper-evident. The genesis step (`seq == 0`) carries `prev_hash:
  null` (the field stays in `cbor_field_order` so the signed layout is stable).
- **`VERDICT_V1`** — an independent judge's verdict, signed by the **judge**
  keypair (must differ from the step `producer`). Binds an optional external
  correctness proof by hash via `proof_ref` + `proof_kind`
  (prm/deterministic/zkml/tee/opml/ocp) — Mnemonic signs *that the verdict
  exists*, never that the model math is sound.
- **`TRAJECTORY_V1`** — anchored summary; `batch_root` is anchored, `prev_root`
  links checkpoints (root-of-roots).

**Two Merkle constructions, one verifier.** `merkle::commitment_root` (recall)
sorts + dedups → commits to a *set* (order-independent). `merkle::trajectory_root`
keeps `seq` order, never sorts/dedups → commits to a *sequence* (reorder changes
the root). Both reuse the same domain-separated leaf/node hashing
(`0x00`/`0x01`), so a single `merkle::verify` checks inclusion proofs from
either. The trajectory root **is** the Arweave bundle manifest root (data items
laid out in `seq` order). These MUST NOT be unified — see the feature's
`decisions.md`. Threat coverage: [`threat-model.md`](threat-model.md).

## Pointers

- **Current Signer abstraction:** [`packages/sdk/src/signer.ts`](../../../packages/sdk/src/signer.ts), Decision 4 of `work/completed/mnemonic-cli/tech-spec.md` (when feature archives).
- **Where Ed25519 is hard-coded:** see "Current state" table above for file refs.
- **COSE alg registry:** [IANA COSE Algorithms](https://www.iana.org/assignments/cose/cose.xhtml#algorithms) — Ed25519 = -8, ES256 = -7, RS256 = -257.
- **Backlog priority:** `work/mnemonic-cli/backlog.md` § "TOP PRIORITY 2 — Crypto-flexibility".
- **Companion concern (bundle size):** `work/mnemonic-cli/backlog.md` § "TOP PRIORITY 3 — Bundle-size optimization" — pure-JS Ed25519 path could be expanded into multi-alg path concurrently.
- **Companion concern (economics):** [`economics.md`](economics.md) — full mode + billing is sequenced before crypto-flex per recommended ordering.
