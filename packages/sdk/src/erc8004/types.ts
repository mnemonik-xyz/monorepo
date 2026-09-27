// TypeScript types for MNEMONIC_FEEDBACK_V1 document and prepareFeedback API.

// ── Document schema ────────────────────────────────────────────────────────

export interface MnemonicAnchor {
  chain: string;
  kind: string;
  ref: string;
}

export interface MnemonicReference {
  schema: string;
  attestation_id: string;
  /** Content hash of the cited attestation — 64 lowercase hex chars. */
  blake3: string;
  /** Base58 Ed25519 public key of the rater's long-lived identity. */
  ed25519_pubkey: string;
  cose_envelope_uri?: string;
  /** Anchor-agnostic shape; slots Ethereum in as {chain:"ethereum",...} with no V2. */
  anchor?: MnemonicAnchor;
}

export interface FeedbackTags {
  tag1?: string;
  tag2?: string;
}

/** The hashed payload — what the Ed25519 key signs. */
export interface FeedbackPayload {
  agentRegistry: string;     // "eip155:<chainId>:0x<reputationRegistryAddress>"
  agentId: string;           // decimal STRING (uint256 exceeds JSON safe range)
  clientAddress: string;     // EIP-55 address; MUST equal msg.sender of giveFeedback tx
  createdAt: string;         // RFC 3339, UTC, second precision
  value: number;             // safe-range integer (int128 on-chain, but doc uses number)
  valueDecimals: number;     // 0–18
  tags?: FeedbackTags;       // optional keys omitted, never null
  endpoint?: string;
  mnemonic: MnemonicReference;
  note?: string;             // optional, ≤ 280 chars
}

export interface Ed25519Proof {
  type: "ed25519";
  kid: string;    // base58 pubkey
  sig: string;    // base64, 64 raw bytes
}

export interface Eip191Proof {
  type: "eip191";
  address: string; // EIP-55
  sig: string;     // 0x hex, 65 bytes
}

export type Proof = Ed25519Proof | Eip191Proof;

export interface MnemonicFeedbackV1 {
  schema: "MNEMONIC_FEEDBACK_V1";
  feedback: FeedbackPayload;
  /** keccak256(JCS(document.feedback)) — what the Ed25519 key signs. */
  payload_hash: string;
  proofs: Proof[];
}

// ── prepareFeedback API ────────────────────────────────────────────────────

export interface PrepareFeedbackInput {
  /** ERC-721 token ID of the rated agent — passed as string or bigint. */
  agentId: string | bigint;
  /** Signed fixed-point score (int128 on chain). */
  value: number | bigint;
  /** Decimal places (0–18). */
  valueDecimals: number;
  /** EIP-55 checksum address of the rater — MUST equal msg.sender of the tx. */
  clientAddress: string;
  /** URI where the feedback document will be hosted before the tx is sent. */
  feedbackUri: string;
  /** The cited Mnemonic evidence. */
  mnemonic: MnemonicReference;
  tags?: FeedbackTags;
  endpoint?: string;
  note?: string;
  /**
   * RFC 3339 timestamp. Caller provides it — no Date.now() inside this
   * function so tests can produce deterministic documents.
   */
  createdAt: string;
  /** EIP-155 chain ID (default: 1 = Ethereum mainnet). */
  chainId?: number;
  /** Override the Reputation Registry address (useful for testnets or forks). */
  registryAddress?: string;
}

/** Minimal signer interface — compatible with LocalSigner. */
export interface FeedbackSigner {
  /** Base58 Ed25519 public key. */
  readonly pubkey: string;
  /** Sign raw bytes, return 64-byte raw Ed25519 signature. */
  sign(bytes: Uint8Array): Promise<Uint8Array>;
}

export interface PrepareFeedbackOpts {
  signer: FeedbackSigner;
  /** Optional EIP-191 proof supplied by the caller's EVM wallet. */
  eip191Proof?: Eip191Proof;
}

export interface OnchainBlock {
  chainId: number;
  /** Reputation Registry address. */
  to: `0x${string}`;
  /** ABI-encoded giveFeedback calldata (4-byte selector + args). */
  data: `0x${string}`;
  value: "0x0";
  functionSignature: string;
  selector: `0x${string}`;
}

export interface PreflightBlock {
  /** The EVM address that MUST send the giveFeedback transaction. */
  requiredSender: `0x${string}`;
  chainId: number;
}

export interface PreparedFeedback {
  document: MnemonicFeedbackV1;
  /** JSON string to upload to feedbackUri before sending the transaction. */
  documentJson: string;
  /** keccak256(JCS(document.feedback)) — the Ed25519-signed hash. */
  payloadHash: `0x${string}`;
  /** keccak256(JCS(document)) — the bytes32 committed on chain. */
  feedbackHash: `0x${string}`;
  feedbackUri: string;
  onchain: OnchainBlock;
  preflight: PreflightBlock;
  warnings: string[];
}

// ── verifyFeedbackDocument API ─────────────────────────────────────────────

export interface VerifyFeedbackInput {
  /** The raw JSON string fetched from feedbackUri. */
  documentJson: string;
  /** feedbackHash bytes32 read from the Reputation Registry (0x hex). */
  onchainFeedbackHash?: string;
  /** msg.sender of the giveFeedback transaction (optional but recommended). */
  onchainSender?: string;
}

export type SenderBindingStatus = "verified" | "mismatch" | "not-checked";

export interface VerifyChecks {
  /** keccak256(JCS(document)) == onchainFeedbackHash */
  feedbackHash: boolean;
  /** keccak256(JCS(feedback)) == document.payload_hash */
  payloadHash: boolean;
  /** Ed25519 signature over signed_message is valid */
  ed25519: boolean;
  /** onchainSender == document.feedback.clientAddress */
  senderBinding: SenderBindingStatus;
}

export interface VerifyFeedbackResult {
  valid: boolean;
  senderBinding: SenderBindingStatus;
  checks: VerifyChecks;
  /** Set when parsing fails or an exception occurs during verification. */
  error?: string;
}
