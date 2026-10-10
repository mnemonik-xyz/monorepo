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
export { parseOperatorList, connectOperator, verifyOperatorIdentity, OPERATOR_PROOF_DOMAIN } from "./operators.js";
export type { OperatorConfig, OperatorList, OperatorProofOptions } from "./operators.js";
export { ArweaveDiscoverySource, DiscoveryError } from "./discovery.js";
export type {
  DiscoverySource, DiscoveryScope, DiscoveryCandidate, DiscoveryPage,
  DiscoveryStatus, DiscoveryDiagnostics,
} from "./discovery.js";
export {
  signRecoveryCheckpoint, verifyRecoveryCheckpoint, checkpointA2ARestoreOptions,
} from "./checkpoint.js";
export type { RecoveryCheckpoint, SignedRecoveryCheckpoint } from "./checkpoint.js";
export { createRecoveryBackup, openRecoveryBackup } from "./backup.js";
export type { RecoveryBackup } from "./backup.js";
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
  A2AArtifact,
  A2AMessage,
  A2APart,
  A2ATask,
  AttestA2AOptions,
  A2ARecipientCard,
  AttestationId,
  Attestation,
  SealedA2AStream,
  Embedder,
  GrantEntry,
  KeypairProvider,
  MnemonicClientConfig,
  OpenMemoryResult,
  ProveResult,
  RecallA2AContextOptions,
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

// ── ERC-8004 reputation feedback ──────────────────────────────────────────
export { prepareFeedback, verifyFeedbackDocument, FeedbackError } from "./erc8004/feedback.js";
export { checkSelfPromotion } from "./erc8004/self-promotion.js";
export type {
  MnemonicFeedbackV1,
  PreparedFeedback,
  PrepareFeedbackInput,
  PrepareFeedbackOpts,
  FeedbackSigner,
  VerifyFeedbackInput,
  VerifyFeedbackResult,
  SenderBindingStatus,
} from "./erc8004/types.js";
export type {
  SelfPromotionCheckResult,
  SelfPromotionStatus,
  CheckSelfPromotionInput,
} from "./erc8004/self-promotion.js";

// ── T14: Sealed A2A DataPart helpers ──────────────────────────────────────
export {
  SEALED_CBOR_MEDIA_TYPE,
  buildSealedDataPart,
  extractSealedDataPart,
  verifyA2AAttestation,
  verifyA2AInner,
} from "./a2a.js";
export type { SealedA2ADataPart, SealedA2APartPayload, OpenedA2A, A2AInnerBinding } from "./a2a.js";

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

export type { A2AIndexStore, A2ARestoreOptions, A2ARestoreReport } from "./types.js";

export * from "./storage.js";
export * from "./storage-portability.js";
