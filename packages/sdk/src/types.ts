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
export type WriteMode = "local" | "participate";

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
