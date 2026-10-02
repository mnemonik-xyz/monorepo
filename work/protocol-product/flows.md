# Information and payment flows

Status: planned logical contracts, not evidence of deployed behavior.
Related: [technical specification](tech-spec.md) and [proof of concept](proof-of-concept.md).

## Save and handoff

```mermaid
sequenceDiagram
    participant A as Author client
    participant O as Operator
    participant S as Storage
    participant B as Recipient client
    Note over A: Build, seal and sign locally
    A->>O: Complete signed bytes and request bindings
    O->>O: Validate author, format and parent
    O->>S: Upload original bytes unchanged
    S-->>O: Locator
    O->>S: Bounded fetch
    S-->>O: Original signed bytes
    O->>O: Compare bytes and verify artifact
    O-->>A: Delivery receipt and payment status
    A-->>B: Locator and authenticated author identity
    B->>S: Fetch original bytes
    S-->>B: Signed artifact
    B->>B: Verify signature, grant and bindings
    Note over B: Open authorized content locally
```

Payer identity does not confer authorship or read access.
An operator transport signature is not the artifact author signature.
Keys and private plaintext remain on clients in the target private path.

## Independent recovery

```mermaid
sequenceDiagram
    participant C as Fresh client
    participant I as Discovery indexer
    participant S as Storage
    Note over C: Load backed-up keys and trusted checkpoint
    alt Context discovery
        C->>I: Scope, expected authors and cursor
        I-->>C: Candidate locators and source status
    else Known locators
        Note over C: Use independent locator manifest
    end
    C->>S: Fetch candidate and parent bytes
    S-->>C: Original signed bytes
    C->>C: Verify authors, grants and graph
    C->>C: Open complete content and rebuild local index
    Note over C: Report complete to pinned heads or explicit gaps
```

The original operator is absent from this flow.
Indexer omissions, outage and lag remain explicit.
Known-locator fetch can bypass discovery.
Storage migration follows [storage-portability-spec.md](storage-portability-spec.md).

## Payment and delivery

```mermaid
sequenceDiagram
    participant C as Payer client
    participant O as Operator
    participant P as Payment rail
    participant S as Storage
    C->>O: Artifact digest, size and backend
    O-->>C: Bound quote and operation ID
    C->>O: Signed bytes and authorization or proof
    O->>O: Validate input and financial replay state
    O->>P: Verify payment
    alt Rail requires settlement before delivery
        P-->>O: Settled payment reference
    else Rail supports delayed settlement
        P-->>O: Accepted authorization
    end
    O->>S: Upload unchanged accepted bytes
    O->>S: Fetch and verify delivered artifact
    S-->>O: Original bytes or delivery error
    alt Verified delivery
        opt Accepted delayed settlement
            O->>P: Settle once
        end
        O-->>C: Verified delivery and financial result
    else Retryable delivery failure
        O-->>C: Pending or retryable status
        Note over C,O: Retry same operation with identical bytes
    else Terminal failure after settlement
        O-->>C: Failed delivery and remedy pending
        Note over O,P: Complete agreed refund or credit with evidence
    end
```

Settlement ordering depends on the rail.
The branches do not represent two charges.
Store financial replay state durably; receipt loss does not erase a settled operation.
Invalid input must fail before irreversible payment where possible.

Quote amount binds size, artifact digest, backend and expiry.
A retry retains operation identity and cannot substitute different bytes.
The baseline requires client resubmission because hosted SQL retains no artifact bytes.
Financial state loss is an operational incident, not permission for blind replay.

Operator payments to storage, gateway and payment providers are separate expenses.
This contract defines no token, buyback, fee router or numeric revenue split.
