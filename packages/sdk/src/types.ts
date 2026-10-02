// Public TypeScript types for @mnemonik-xyz/sdk.
//
// Mirrors tech-spec § Data Models. These are the only types intended for
// consumer use; internal helpers stay unexported (re-exports happen in
// `src/index.ts`).

import type { Keypair } from "./keypair.js";

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
  a2aGatewayUrl?: string;
  a2aIndexUrl?: string;
  a2aIndexFlavour?: "irys" | "arweave";
  a2aIndex?: A2AIndexStore;
  baseUrl: string;
  signer: SignerInterface;
  jwt?: string;
  /**
   * Optional fetch override — primarily for testing. Defaults to the
   * runtime's global `fetch`. Must conform to the standard Fetch API.
   */
  fetch?: typeof fetch;
  /**
   * Optional lazy keypair source for `signMemory`. The client calls it only
   * when the server asks for a client signature (`awaiting_signature`).
   * See {@link MnemonicClient.setKeypairProvider}.
   */
  keypairProvider?: KeypairProvider;
  /**
   * Optional access-token refresher. See
   * {@link MnemonicClient.setTokenRefresher}.
   */
  tokenRefresher?: TokenRefresher;
}

/**
 * Per-request write mode for `signMemory`.
 *
 * - `"local"`: the server stores the memory for the caller only. There is
 *   no on-chain anchor, no charge and no client signature. The call works
 *   from the public key alone.
 * - `"participate"`: the client signs the canonical bundle (COSE_Sign1)
 *   and the server anchors it on Arweave and Solana. This needs the
 *   private key.
 */
/**
 * `"anchored"` is the canonical spelling. `"participate"` is the deprecated
 * pre-2026-09-27 name for the same mode, accepted on input for one release
 * so existing callers keep working. The server never returns it.
 */
export type WriteMode = "local" | "anchored" | "participate";

/**
 * Lazy keypair source. The client calls it only when it must make a
 * signature, so a caller can defer an OS-keychain read (and its prompt)
 * until then. It can return the keypair or a promise of it.
 */
export type KeypairProvider = () => Keypair | Promise<Keypair>;

/**
 * Access-token refresher. It returns a fresh access JWT, or `undefined`
 * when it cannot refresh (the client then keeps the current token). An
 * error it throws goes to the caller of the tool method.
 */
export type TokenRefresher = () => Promise<string | undefined>;

/** Caller-supplied options for `signMemory`. */
export interface SignMemoryOptions {
  tags?: string[];
  /**
   * Write mode, sent to the server as `mode`. If you omit it, the server
   * uses its own default and the client needs a keypair before the call
   * (legacy behavior).
   */
  mode?: WriteMode;
}

/** Server response shape from `signMemory`. */
export interface SignMemoryResult {
  attestationId: string;
  signedAt: string;
  /**
   * `stored`: the server stored the memory without a client signature (a
   * `local` write). `signed` / `pending` / `anchored`: the client signed
   * the bundle. `anchored` means that the on-chain anchor is confirmed.
   */
  status: "stored" | "signed" | "pending" | "anchored";
  /** Write mode that the server applied, when the server reports it. */
  writeMode?: WriteMode;
  /** Server content_hash echo (hex blake3 of canonical CBOR). */
  contentHash?: string;
  arweaveTx?: string;
  solanaTx?: string;
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
  [key: string]: unknown;
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
  /** Unmodified server evidence; inclusion is not a completeness guarantee. */
  evidence?: Record<string, unknown>;
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

// ── Sealed-memory types ────────────────────────────────────────────────────

/**
 * Write mode for `sealMemory`.
 *
 * - `"anchor"`: store on-chain (Arweave / Solana anchor); posts to
 *   `/api/anchor-sealed`. Requires an active session.
 * - `"store"`: store off-chain in the hosted service; posts to
 *   `/api/store-sealed`. Cheaper, still E2E encrypted.
 */
export type SealMode = "anchor" | "store";

/** Options for `sealMemory`. */
export interface SealMemoryOptions {
  mode: SealMode;
  tags?: string[];
  /** Local embedder for client-side matching; embeddings are not uploaded.
   *  Pass an embedder explicitly when calling `recallSealed`. */
  embedder?: Embedder;
}

/** Result of `sealMemory`. */
export interface SealMemoryResult {
  /** blake3 content hash (hex) of the sealed outer CBOR, i.e. the memory's
   *  globally unique identifier. */
  memoryHash: string;
  /** Original signed bytes to retain for independent recovery and exact retries. */
  signedBytes?: Uint8Array;
  outerCbor?: Uint8Array;
  locator?: string;
  receipt?: Record<string, unknown>;
}

/** Result of `openMemory`. */
export interface OpenMemoryResult {
  /** The plaintext inner memory content. */
  content: string;
  /** The raw inner JSON bytes, if needed by the caller. */
  innerJson: Uint8Array;
}

/** Identifies a reader in `share()`. */
export type ShareTarget = { kid: string; x25519Pub: Uint8Array } | "link";

/** Result of `share()`. */
export type ShareResult =
  | { type: "grant"; grantCbor: Uint8Array }
  | { type: "link"; url: string };

/** One grant entry from `listGrants`. */
export interface GrantEntry {
  grantId: string;
  memoryHash: string;
  reader?: string;
  createdAt: string;
}

/** Options for `recallSealed`. */
export interface RecallSealedOptions {
  topK?: number;
  embedder?: Embedder;
}

/** One hit from `recallSealed`. */
export interface SealedHit {
  memoryHash: string;
  similarity: number;
  /** Present only after the hit is opened. */
  content?: string;
}

/**
 * Pluggable embedding function. Given a query string, returns a float32
 * vector of arbitrary dimension (must match the stored vectors).
 *
 * The default implementation posts to `POST /api/embed` on the MnemonicClient's
 * base URL.
 */
export interface Embedder {
  embed(text: string): Promise<Float32Array>;
}

// ── A2A types (Task 7) ────────────────────────────────────────────────────────
// Field names match A2A v1.0.0-rc.

/**
 * A part of an A2A message — text, file, or data.
 *
 * @see https://google.github.io/A2A/spec/
 */
export type A2APart =
  | { type: "text"; text: string }
  | {
      type: "file";
      file: {
        name?: string;
        mimeType?: string;
        bytes?: string;
        uri?: string;
      };
    }
  | { type: "data"; data: Record<string, unknown> };

/**
 * An A2A task — the top-level unit of work exchanged between agents.
 *
 * Corresponds to `Task` in the A2A v1.0.0-rc specification.
 */
export interface A2ATask {
  /** Unique task identifier. */
  id: string;
  /** Context identifier grouping related tasks / messages. */
  contextId?: string;
  /** Task status. */
  status: {
    state:
      | "submitted"
      | "working"
      | "input-required"
      | "completed"
      | "canceled"
      | "failed"
      | "unknown";
    message?: A2AMessage;
    timestamp?: string;
  };
  /** Task history (prior messages). */
  history?: A2AMessage[];
  /** Artifacts produced by the task. */
  artifacts?: A2AArtifact[];
  /** Agent-specific metadata. */
  metadata?: Record<string, unknown>;
}

/**
 * An A2A message — a single turn in an agent conversation.
 *
 * Corresponds to `Message` in the A2A v1.0.0-rc specification.
 */
export interface A2AMessage {
  /** Unique message identifier. */
  messageId: string;
  /** The task this message belongs to. */
  taskId?: string;
  /** Context identifier. */
  contextId?: string;
  /** Sender role. */
  role: "user" | "agent";
  /** Message content parts. */
  parts: A2APart[];
  /** Agent-specific metadata. */
  metadata?: Record<string, unknown>;
  /** ISO-8601 timestamp. */
  timestamp?: string;
}

/**
 * An A2A artifact — output produced by an agent task.
 *
 * Corresponds to `Artifact` in the A2A v1.0.0-rc specification.
 */
export interface A2AArtifact {
  /** Artifact identifier. */
  artifactId: string;
  /** The task that produced this artifact. */
  taskId?: string;
  /** Human-readable name. */
  name?: string;
  /** Description of the artifact. */
  description?: string;
  /** Artifact content parts. */
  parts: A2APart[];
  /** Agent-specific metadata. */
  metadata?: Record<string, unknown>;
  /** ISO-8601 timestamp. */
  createdAt?: string;
  /** Artifact index (when multiple artifacts per task). */
  index?: number;
  /** Whether this is the last artifact (for streaming). */
  lastChunk?: boolean;
  /** Whether the artifact should be appended to a previous one. */
  append?: boolean;
}

/** Opaque attestation identifier returned by all attest* methods. */
export type AttestationId = string;

/** Options for `attestA2ATask`, `attestA2AMessage`, `attestA2AArtifact`. */
export interface A2ARecipientCard {
  /** Complete AgentCard, including detached JWS signatures. */
  card: Record<string, unknown>;
  /** Pinned card-signing Ed25519 key. Obtain this from trusted configuration. */
  trustedCardSigner: string;
}

export interface AttestA2AOptions {
  mode?: "local" | "anchored";
  prevLocator?: string;
  /** Seal in the client for these recipients. Omit for a plain signed binding. */
  sealed?: { recipients: A2ARecipientCard[]; chunkSize?: number };
  /** Link to a prior attestation in the same context. */
  prevId?: string;
}

/**
 * A single attestation record returned by `recallA2AContext`.
 */
/** Signed completed-stream manifest; wire field names match the protocol. */
export interface SealedA2AStream {
  stream_id: string;
  nonce_prefix: string;
  header_hash: string;
  head: string;
  chunks: {index:number; last:boolean; prev_hash:string; ciphertext:string; hash:string}[];
}

export interface Attestation {
  locator?: string;
  attestationId: string;
  contextId?: string;
  prevId?: string | null;
  coseEnvelopeHex?: string;
  contentHash?: string;
  signerPubkey?: string;
  sealed?: boolean;
  sealedPayload?: { sealed: string; grants: string[] };
  stream?: SealedA2AStream;
  /** "task" | "message" | "artifact" */
  kind: "task" | "message" | "artifact";
  /** ISO-8601 timestamp of when the attestation was signed. */
  signedAt: string;
  /** The attested object (verbatim from the server). */
  payload: Record<string, unknown>;
}

/** Options for `recallA2AContext`. */
export interface RecallA2AContextOptions {
  mode?: "local" | "anchored";
  /** Filter sealed or plaintext records. Omit to retrieve both. */
  sealed?: boolean;
  /** Max number of attestations to return. Default: server-side. */
  limit?: number;
  /** Filter by object kind. Defaults to `"all"`. */
  kind?: "task" | "message" | "artifact" | "all";
}

/** Agent-owned cache of original signed artifacts. Default implementation is session-only. */
export interface A2AIndexStore { list(): Promise<Attestation[]>; put(row:Attestation): Promise<void>; }
export interface A2ARestoreOptions {
 expectedAuthors:string[]; heads?:string[]; maxPages?:number; maxCandidates?:number;
 /** Replace discovery, or use false for explicit-locator/local recovery only. */
 discoverySource?:import('./discovery.js').DiscoverySource | false;
 checkpoint?:{scope:string;cursor?:string;seenCursors?:string[];staged?:Attestation[]}; signal?:AbortSignal;
 parentLocators?:Record<string,{locator:string;author:string}>;
}
export interface A2ARestoreReport {
 attestations:Attestation[]; scanExhausted:boolean; budgetExhausted:boolean;
 source:import('./discovery.js').DiscoveryDiagnostics;
 missingParents:string[]; invalidCandidates:{locator:string;reason:string}[];
 completeToHeads:boolean; completeness:'unknown'|'complete_to_heads';
 checkpoint?:{scope:string;cursor?:string;seenCursors?:string[];staged?:Attestation[]}; error?:string;
}
