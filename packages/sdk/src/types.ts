// Public TypeScript types for @mnemonik-xyz/sdk.
//
// Mirrors tech-spec § Data Models. These are the only types intended for
// consumer use; internal helpers stay unexported (re-exports happen in
// `src/index.ts`).

/**
 * Pluggable signing primitive for raw Ed25519 signatures over arbitrary
 * byte payloads.
 *
 * Phase 1 ships `LocalSigner` (in-memory keypair, signs via WASM
 * `sign_challenge`). Future implementations (e.g. `TurnkeySigner`,
 * `WebAuthnSigner`) are drop-in replacements; they MUST pass the contract
 * suite at `test/signer-contract.ts`.
 *
 * Note: this interface is **NOT** for COSE_Sign1 envelope construction.
 * COSE happens in `client.ts::signMemory` via the `cose.ts` wrapper around
 * WASM `sign_cose_payload`. Keeping the byte-signer surface generic means
 * non-Ed25519 signers (e.g. WebAuthn over P-256) can plug in without
 * mixing concerns.
 */
export interface SignerInterface {
  /** Base58-encoded Ed25519 public key. Non-empty. */
  readonly pubkey: string;
  /**
   * Produce a 64-byte raw Ed25519 signature over `bytes`.
   *
   * Implementations MUST reject zero-length input (throw a `UserError`)
   * and MUST be deterministic (Ed25519 RFC 8032).
   */
  sign(bytes: Uint8Array): Promise<Uint8Array>;
}

/**
 * Constructor configuration for `MnemonicClient`.
 *
 * `baseUrl` should be the origin of the hosted MCP server, e.g.
 * `https://mcp.mnemonik.xyz`. The SDK appends path segments (`/mcp`,
 * `/api/sign-callback`, `/api/pending/...`) — do NOT include a trailing
 * slash or path here.
 *
 * `jwt` is optional at construction time so SDK consumers can build a
 * client first, run the OAuth flow, then call `setJwt(...)`. The signer
 * is required: `signMemory` always needs it for the COSE step.
 */
export interface MnemonicClientConfig {
  baseUrl: string;
  signer: SignerInterface;
  jwt?: string;
  /**
   * Optional fetch override — primarily for testing. Defaults to the
   * runtime's global `fetch`. Must conform to the standard Fetch API.
   */
  fetch?: typeof fetch;
  /**
   * Optional wallet/payment bridge. When a paid participate callback returns
   * HTTP 402, the SDK passes the typed quote here and retries the exact same
   * client-signed operation with the returned authorization.
   */
  paymentHandler?: PaymentHandler;
}

export type PaymentAuthorization =
  | {
      /** `session` is the preferred product name; `stake` is the V1 wire alias. */
      scheme: "session" | "stake";
      session_id: string;
      payer_wallet: string;
      authorization?: unknown;
    }
  | {
      scheme: "exact";
      payer_wallet: string;
      authorization: unknown;
    };

export interface PaymentOperationScope {
  workspace_hash?: string;
  visibility: "private" | "public";
  action: "manual" | "pre_compaction" | "session_end";
}

export interface PaymentOperationBinding {
  version: 1;
  operation_id: string;
  payer_subject: string;
  payer_wallet: string;
  artifact_hash: string;
  amount: string;
  asset: string;
  network: string;
  pay_to: string;
  expires_at: string;
  nonce: string;
  scope: PaymentOperationScope;
}

export interface PaymentReceipt {
  operation_id: string;
  scheme: "stake" | "exact";
  status: "settled";
  binding_digest: string;
  payer_wallet: string;
  amount: string;
  asset: string;
  network: string;
  pay_to: string;
  settlement_tx?: string;
  settled_at: string;
  receipt: unknown;
}

export interface PaymentQuote {
  amount: string;
  asset: string;
  network: string;
  pay_to: string;
  expires_at: string;
  accepts: Array<Record<string, unknown> & { scheme: "stake" | "exact" }>;
}

export interface SignedPaymentChallenge {
  operationId: string;
  artifactHash: string;
  binding: PaymentOperationBinding;
  bindingDigest: string;
  /** Initial 402s are provisional until the payer wallet is connected. */
  bindingStatus: "provisional" | "final";
  workspace?: string;
  refreshed: boolean;
  quote: PaymentQuote;
  /** Opaque callback material used by `resumePaidMemory`; do not modify. */
  callback: {
    correlation_id: string;
    cose_signed_bytes: string;
    signer_pubkey: string;
  };
}

export interface PreparedPaymentOperation {
  operationId: string;
  binding: PaymentOperationBinding;
  bindingDigest: string;
}

export type PreparePaymentOperation = (
  payerWallet: string
) => Promise<PreparedPaymentOperation>;

export type PaymentHandler = (
  challenge: SignedPaymentChallenge,
  /** Finalize the binding after wallet connection, before signing payment. */
  prepare: PreparePaymentOperation
) => Promise<PaymentAuthorization>;

/** Caller-supplied options for `signMemory`. */
export interface SignMemoryOptions {
  tags?: string[];
  mode?: "local" | "participate";
  visibility?: "private" | "public";
  checkpointType?: "manual" | "pre_compaction" | "session_end";
  workspace?: string;
  /** Reuse an already approved session or supply an exact x402 authorization. */
  payment?: PaymentAuthorization;
}

/** Server response shape from `signMemory` after the callback completes. */
export interface SignMemoryResult {
  attestationId: string;
  signedAt: string;
  status: "signed" | "pending" | "anchored";
  /** Server content_hash echo (hex blake3 of canonical CBOR). */
  contentHash?: string;
  arweaveTx?: string;
  solanaTx?: string;
  operationId?: string;
  paymentReceipt?: PaymentReceipt;
}

export interface PaidOperationStatus {
  operationId: string;
  status:
    | "awaiting_payment"
    | "payment_authorizing"
    | "payment_ready"
    | "anchoring"
    | "verifying_delivery"
    | "anchored"
    | "payment_failed"
    | "delivery_retryable";
  binding: PaymentOperationBinding;
  bindingDigest: string;
  bindingStatus: "provisional" | "final";
  workspace?: string;
  receipt?: PaymentReceipt;
  attestationId?: string;
  solanaTx?: string;
  arweaveTx?: string;
  lastError?: string;
}

/**
 * Result of `verify` — discriminated union over the three terminal states
 * the server can return.
 */
export type VerifyResult =
  | {
      status: "verified";
      signer: string;
      arweaveTx?: string;
      solanaTx?: string;
    }
  | { status: "tampered"; signer: string; reason: string }
  | { status: "not_found" };

/** A single hit from `recall`. Server-defined fields are passed through. */
export interface RecallHit {
  attestationId: string;
  content: string;
  similarity: number;
  signedAt?: string;
  tags?: string[];
}

export interface RecallResult {
  hits: RecallHit[];
  /** Total matching attestations on the server side. */
  total: number;
}

/**
 * Result of `whoami`. Returns the **server's** identity — the CLI does not
 * use this (Decision 14) but SDK consumers may want a generic
 * server-pubkey check.
 */
export interface WhoamiResult {
  serverPubkey?: string;
  serverDid?: string;
  /** Anything else the server sends — preserved for forward compat. */
  raw: Record<string, unknown>;
}

/** Result of `proveIdentity` — server-side challenge-sign helper. */
export interface ProveResult {
  pubkey: string;
  challenge: string;
  signature: string;
  did?: string;
  raw: Record<string, unknown>;
}
