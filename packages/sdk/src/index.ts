// @mnemonik-xyz/sdk — public entrypoint.
//
// Public API surface (Phase 1, Wave 1+2):
//   - MnemonicClient (5 tool methods, signMemory uses pending-bundle flow)
//   - Signer interface + LocalSigner
//   - Keypair helpers
//   - Typed error classes + redactJWT helper
//   - Public TS types
//   - OAuth 2.1 + PKCE primitives (T3 — see oauth.ts)
//
// Internal helpers (`wasm.ts`, low-level encoders) are intentionally NOT
// re-exported. The CLI imports only from this barrel.

// ── T2: client + signer + keypair + types + errors ──────────────────────────
export { MnemonicClient } from "./client.js";
export { coseSignPayload } from "./cose.js";
export {
  AuthError,
  IntegrityError,
  MnemonicError,
  ServerError,
  UserError,
  redactJWT,
} from "./errors.js";
export { Keypair } from "./keypair.js";
export type { KeypairJson } from "./keypair.js";
export { LocalSigner } from "./signer.js";
export type { Signer } from "./signer.js";
export type {
  MnemonicClientConfig,
  ProveResult,
  RecallHit,
  RecallResult,
  SignMemoryOptions,
  SignMemoryResult,
  SignerInterface,
  VerifyResult,
  WhoamiResult,
} from "./types.js";

// ── T3: OAuth surface ──────────────────────────────────────────────────────
export {
  buildAuthorizeUrl,
  exchangeCodeForToken,
  parseJwtPayload,
  generatePkceVerifier,
  pkceChallenge,
  randomState,
  pendingAuthSessions,
} from "./oauth.js";
export type {
  BuildAuthorizeUrlInput,
  BuildAuthorizeUrlResult,
  ExchangeCodeForTokenInput,
  ExchangeCodeForTokenResult,
  JwtPayload,
  PendingAuthSession,
} from "./oauth.js";
