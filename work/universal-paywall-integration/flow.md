---
updated: 2026-07-15
status: implementation-reference
scope: phase-1-exact-payment
---

# Universal Paywall exact-payment flow

This document describes the Phase 1 exact-payment journey as implemented and
the restart-resume boundary being completed before wallet identity linking.
Local writes remain free and do not enter this flow.

## Primary journey

```mermaid
sequenceDiagram
    autonumber
    participant C as Mnemonic client / IDE
    participant M as Mnemonic MCP
    participant DB as Mnemonic SQLite
    participant W as Approval webapp + wallet
    participant UP as Universal Paywall
    participant A as Arweave / Irys
    participant S as Solana

    C->>M: sign_memory(mode=participate, content)
    M->>M: embed → TurboQuant compress → canonical CBOR
    M-->>C: unsigned CBOR + correlation_id
    C->>C: COSE_Sign1 over exact canonical CBOR
    C->>M: sign-callback(correlation_id, COSE, signer)
    M->>M: verify COSE and canonical-CBOR content hash
    M->>DB: stage COSE + delivery context
    M->>M: artifact_hash = blake3(domain || exact COSE bytes)
    M-->>C: 428 wallet-link challenge
    C->>W: open wallet_link_url and sign EIP-191 challenge
    W->>M: verify wallet link signature
    M->>M: recover EVM wallet address
    C->>M: same sign-callback(correlation_id, same COSE)
    M->>UP: create_quote(operation binding)
    UP-->>M: immutable exact quote
    M-->>C: 402 awaiting_payment + approval_url
    C->>W: open approval_url
    W->>W: EIP-3009 authorization (wallet)
    W->>UP: settle exact authorization
    UP-->>W: signed payment receipt
    C->>M: same sign-callback(correlation_id, same COSE)
    M->>UP: status(operation_id)
    UP-->>M: settled receipt
    M->>A: store exact COSE envelope
    M->>S: anchor canonical-CBOR content hash + Arweave id
    M->>DB: save attestation + delivery receipt
    M-->>C: anchored receipt and recall identifiers
```

The payment binding is deliberately the signed COSE envelope, not raw editor
content and not only the unsigned canonical CBOR. Thus it commits to the
TurboQuant-compressed embedding inside the canonical artifact, the payload,
the protected headers, and the client signer.

## Persistent state boundaries

```mermaid
flowchart LR
    U[Unsigned canonical CBOR<br/>in pending bundle]
    V[Verified COSE envelope]
    Q[Exact quote]
    R[Provider receipt]
    D[Arweave + Solana delivery receipt]

    U -->|client signs| V
    V -->|stage| AS[(paid_artifact_staging)]
    V -->|stage context| DC[(paid_artifact_delivery_context)]
    V -->|hash| Q
    Q --> PO[(paid_operations)]
    R --> PO
    D --> PO
    D --> AT[(attestations)]

    style AS fill:#e8f1ff,stroke:#2563eb
    style DC fill:#e8f1ff,stroke:#2563eb
    style PO fill:#e8f1ff,stroke:#2563eb
    style AT fill:#e8f1ff,stroke:#2563eb
```

- `paid_operations` stores only operation metadata, quote metadata, state,
  and receipts; it never stores private artifact bytes.
- The dedicated staging tables hold the verified COSE envelope and private
  delivery context under the same local SQLite trust boundary used for private
  attestations. They are needed only until delivery is confirmed or abandoned.
- Universal Paywall remains the independent source of truth for settlement.
  Arweave and Solana remain the independent evidence of completed delivery.

## Wallet link requirement

Before a quote, Mnemonic issues an EIP-191 `personal_sign` message bound to
the opaque Mnemonic subject hash, operation id, configured EVM chain, random
nonce, and five-minute expiry. Mnemonic recovers the signer address server
side and stores the verified link as single-use metadata. The Universal Paywall
binding uses that recovered address; `UNIVERSAL_PAYWALL_PAYER_WALLET` is no
longer used as the production quote source.

## Restart-resume path

```mermaid
flowchart TD
    X[MCP restart after client signature] --> Y{Payment settled?}
    Y -->|No| Z[Read staged COSE + context<br/>return same approval URL / operation status]
    Y -->|Yes| P[Read provider receipt by operation_id]
    P --> C[Recover staged COSE + delivery context]
    C --> A[Anchor exact staged COSE]
    A --> V[Verify delivery]
    V --> R[Persist attestation and delivery receipt]

    Z --> W[Wallet settles exact quote]
    W --> P
```

The recovery route must never create a replacement quote, change the signed
artifact, request another wallet authorization, or re-embed content.

## Current limits and follow-on work

- Wallet-to-Mnemonic-subject binding is not yet implemented; the configured
  payer wallet remains a development-only identity source. This is Task 03.
- The legacy `/api/authorization` endpoint still exists but the active E2E
  flow no longer calls it. Task 04 removes it and adds authenticated status /
  resume surfaces.
- The operation lease, quote-expiry, concurrent callback, delivery failure,
  and provider/MCP restart cases require explicit integration tests in Task 06
  before production readiness.
