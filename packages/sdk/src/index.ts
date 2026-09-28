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
  IdentityRequiresKeystore,
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
  Embedder,
  GrantEntry,
  KeypairProvider,
  MnemonicClientConfig,
  OpenMemoryResult,
  ProveResult,
  RecallHit,
  RecallResult,
  RecallSealedOptions,
  SealMemoryOptions,
  SealMemoryResult,
  SealMode,
  SealedHit,
  ShareResult,
  ShareTarget,
  SignMemoryOptions,
  SignMemoryResult,
  SignerInterface,
  TokenRefresher,
  VerifyResult,
  WhoamiResult,
  WriteMode,
} from "./types.js";

// ── T3: OAuth surface ──────────────────────────────────────────────────────
export {
  buildAuthorizeUrl,
  exchangeCodeForToken,
  loginWithIdentity,
  parseJwtPayload,
  refreshAccessToken,
  generatePkceVerifier,
  pkceChallenge,
  randomState,
  pendingAuthSessions,
  BROWSERLESS_REDIRECT_URI,
} from "./oauth.js";
export type {
  BuildAuthorizeUrlInput,
  BuildAuthorizeUrlResult,
  ExchangeCodeForTokenInput,
  ExchangeCodeForTokenResult,
  JwtPayload,
  LoginWithIdentityInput,
  LoginWithIdentityResult,
  PendingAuthSession,
  RefreshAccessTokenInput,
  RefreshAccessTokenResult,
} from "./oauth.js";
