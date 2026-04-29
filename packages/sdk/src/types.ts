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
 * `https://mc.mnemonik.xyz`. The SDK appends path segments (`/mcp`,
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
}

/** Caller-supplied options for `signMemory`. */
export interface SignMemoryOptions {
  tags?: string[];
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
