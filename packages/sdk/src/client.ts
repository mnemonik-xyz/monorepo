import { boundedBody, decodeAttestation, fetchAttestation, importA2AAttestation, restoreA2AContext } from "./a2a-recovery.js";
// MnemonicClient — stateless wrapper over the hosted MCP HTTP surface.
//
// 5 tool methods (whoami, signMemory, recall, verify, proveIdentity) plus
// the canonical pending-bundle handling for signMemory.
//
// **Critical:** signMemory ALWAYS handles the pending-bundle response shape
// (Decision 7 / completeness validator). The hosted server has no inline-
// signed code path for HTTP+JWT clients. Flow:
//   1. POST /mcp tools/call mnemonic_sign_memory  → `{correlation_id, ...}`
//   2. GET /api/pending/{correlation_id}          → canonical-CBOR bytes
//   3. coseSignPayload(cbor, keypair)             → COSE_Sign1 envelope
//   4. POST /api/sign-callback (NO JWT)           → `{attestation_id, ...}`
//
// Step 4 carries no Bearer token — capability auth via correlation_id +
// signer_pubkey + cryptographic chain (server validates signer matches the
// stored jwt_sub for that correlation_id, AND COSE_Sign1 verifies against
// signer_pubkey).
//
// Explicit HTTP `mode: "local"` is rejected by current servers. Legacy
// responses remain readable for migration; client-owned sealed storage uses
// sealMemory({ mode: "store" }) and caller persistence of its returned bytes.
//
// Access-token refresh (issue #33): with a `tokenRefresher`, `callTool`
// renews the JWT before it expires and retries once after a 401/403.
//
// Sealed-memory extensions (T7):
//   sealMemory, openMemory, share, importLink, listGrants, recallSealed.

import {
  attestA2AArtifact,
  attestA2AMessage,
  attestA2ATask,
  recallA2AContext,
  openA2AAttestation,
  type OpenedA2A,
} from "./a2a.js";
import { coseSignPayload } from "./cose.js";
import {
  AuthError,
  IntegrityError,
  MnemonicError,
  ServerError,
  UserError,
  redactJWT,
} from "./errors.js";
import type { Keypair, KeypairJson } from "./keypair.js";
import { readJwtExp } from "./oauth.js";
import type {
  A2AArtifact,
  A2AMessage,
  A2ATask,
  AttestA2AOptions,
  AttestationId,
  Attestation,
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
import { loadWasm } from "./wasm.js";

/**
 * Refresh the JWT when it expires within this window, so a request never
 * reaches the server with a token that expires in flight. This matters
 * for `mnemonic_recall`: the server treats an expired token on that tool
 * as anonymous (public pool only) instead of returning 401.
 */
const REFRESH_SKEW_MS = 60_000;

/**
 * Stateless HTTP client for the hosted MCP server.
 *
 * Construction does no I/O. The signer is required even for read-only
 * methods so a single client can be reused for sign + recall flows; if
 * you only need recall/verify and don't have a keypair yet, pass any
 * concrete `SignerInterface` (its `sign` method won't be invoked).
 */
export class MnemonicClient {
  private readonly baseUrl: string;
  private readonly sealedLocal = new Map<string, { outerCbor: Uint8Array; signedBytes: Uint8Array }>();
  private readonly signer: SignerInterface;
  private jwt: string | undefined;
  private readonly fetchImpl: typeof fetch;
  /**
   * Optional keypair — needed only for `signMemory` (the COSE step needs
   * the JSON form, which `Signer.pubkey` can't provide). When the client
   * is built from an arbitrary `Signer`, `signMemory` will throw `UserError`.
   * `LocalSigner` consumers should call `setKeypair(keypair)` before
   * `signMemory`.
   */
  private keypairJson: KeypairJson | null = null;
  /** Lazy keypair source — see {@link setKeypairProvider}. */
  private keypairProvider: KeypairProvider | null = null;
  /** Memoised provider result (reset on failure or a new provider). */
  private providedKeypair: Promise<KeypairJson> | null = null;
  /** Access-token refresher — see {@link setTokenRefresher}. */
  private tokenRefresher: TokenRefresher | null = null;
  /** Single in-flight refresh, shared by concurrent tool calls. */
  private refreshInFlight: Promise<string | undefined> | null = null;

  private a2aIndex!: import("./types.js").A2AIndexStore;
  private a2aGatewayUrl!: string;
  private a2aDiscoveryConfig!: {url:string;flavour:'irys'|'arweave'};
  setA2AIndexStore(store:import("./types.js").A2AIndexStore):void {this.a2aIndex=store;}
  _a2aIndexStore(){return this.a2aIndex;}
  _a2aGateway(){return this.a2aGatewayUrl;}
  _a2aDiscovery(){return this.a2aDiscoveryConfig;}
  async _a2aExternal(url:string,init:RequestInit={}):Promise<Response>{
    return this.fetchImpl(url,{...init,redirect:'error',credentials:'omit',signal:init.signal?AbortSignal.any([init.signal,AbortSignal.timeout(10000)]):AbortSignal.timeout(10000)});
  }
  importA2AAttestation(locator:string,author:string){return importA2AAttestation.call(this,locator,author);}
  restoreA2AContext(context:string,opts:import("./types.js").A2ARestoreOptions){return restoreA2AContext.call(this,context,opts);}
  constructor(config: MnemonicClientConfig) {
    if (!config.baseUrl || !/^https?:\/\//.test(config.baseUrl)) {
      throw new UserError(
        "MnemonicClient: baseUrl must be an absolute http(s) URL"
      );
    }
    if (!config.signer || typeof config.signer.sign !== "function") {
      throw new UserError("MnemonicClient: signer is required");
    }
    // Strip trailing slash so path concatenation produces e.g.
    // `https://host/mcp` not `https://host//mcp`.
    this.baseUrl = config.baseUrl.replace(/\/+$/, "");
    this.signer = config.signer;
    const rows=new Map<string,Attestation>();
    this.a2aIndex=config.a2aIndex??{list:async()=>[...rows.values()].map(r=>structuredClone(r)),put:async row=>{rows.set(row.attestationId,structuredClone(row));}};
    this.a2aGatewayUrl=(config.a2aGatewayUrl??'https://gateway.irys.xyz').replace(/\/+$/,'');
    if(config.a2aIndexFlavour!==undefined&&!['irys','arweave'].includes(config.a2aIndexFlavour))throw new UserError('invalid A2A index flavour');
    this.a2aDiscoveryConfig={url:config.a2aIndexUrl??'https://uploader.irys.xyz/graphql',flavour:config.a2aIndexFlavour??'irys'};
    for(const url of [this.a2aGatewayUrl,this.a2aDiscoveryConfig.url])if(!/^https?:\/\//.test(url))throw new UserError('invalid external A2A endpoint');
    if (config.jwt !== undefined) this.jwt = config.jwt;
    this.fetchImpl = config.fetch ?? globalThis.fetch.bind(globalThis);
    if (config.keypairProvider) this.keypairProvider = config.keypairProvider;
    if (config.tokenRefresher) this.tokenRefresher = config.tokenRefresher;
  }

  /**
   * Set or replace the JWT after construction (e.g. after the OAuth login
   * flow finishes). Pass `undefined` to detach the current token.
   *
   * @param jwt - HS256-signed bearer token, or `undefined` to clear.
   * @returns void.
   */
  setJwt(jwt: string | undefined): void {
    this.jwt = jwt;
  }

  /**
   * Bind a `Keypair` so that `signMemory` can produce the COSE envelope.
   * Required only for sign flows; `recall`, `verify`, `whoami`, and
   * `proveIdentity` do not need it.
   *
   * @param keypair - The local Ed25519 keypair to bind.
   * @returns void.
   */
  setKeypair(keypair: Keypair): void {
    this.keypairJson = keypair.toJSON();
  }

  /**
   * Bind a lazy keypair source. `signMemory` calls it only when the server
   * asks for a client signature (a `participate` write, or a legacy write
   * without `mode`), and at most once per client: the result is memoised.
   * A `local` write never calls it. Use this to defer an OS-keychain read
   * until the keypair is really needed. A keypair bound with
   * {@link setKeypair} takes priority.
   *
   * @param provider - Returns the keypair (or a promise of it); `null`
   *                   removes the provider.
   * @returns void.
   */
  setKeypairProvider(provider: KeypairProvider | null): void {
    this.keypairProvider = provider;
    this.providedKeypair = null;
  }

  /**
   * Bind an access-token refresher (issue #33). The client calls it:
   *
   * - before a tool call, when there is no JWT or the JWT expires within
   *   60 s (so `recall` never silently falls back to anonymous);
   * - once after a tool call fails with 401 / 403, and then retries it once.
   *
   * Concurrent calls share one refresh. Build a refresher on top of
   * `refreshAccessToken` and persist the rotated refresh token each time.
   *
   * @param refresher - Returns a fresh JWT, or `undefined` if it cannot
   *                    refresh; `null` removes the refresher.
   * @returns void.
   */
  setTokenRefresher(refresher: TokenRefresher | null): void {
    this.tokenRefresher = refresher;
  }

  // ------------------------------------------------------------------------
  // Tool methods
  // ------------------------------------------------------------------------

  /**
   * Call the server's `mnemonic_whoami` MCP tool.
   *
   * Returns the **server's** identity (its Ed25519 pubkey + DID), NOT the
   * caller's. Per Decision 14 the CLI implements its own client-side
   * `whoami` instead of calling this; SDK consumers may still find it
   * useful for a generic server-pubkey check.
   *
   * @returns The decoded `WhoamiResult` plus the raw server payload.
   * @throws `AuthError` on 401/403, `ServerError` on 5xx / network failure.
   */
  async whoami(): Promise<WhoamiResult> {
    const result = await this.callTool("mnemonic_whoami", {});
    const raw = isRecord(result) ? result : {};
    return {
      ...(typeof raw.server_pubkey === "string"
        ? { serverPubkey: raw.server_pubkey }
        : {}),
      ...(typeof raw.server_did === "string"
        ? { serverDid: raw.server_did }
        : {}),
      raw,
    };
  }

  /**
   * Save a memory. With `mode: "local"` the server stores it as a hash-only
   * row and returns it at once (`status: "stored"`): no keypair is needed
   * and the keypair provider is not called. Otherwise (and whenever the
   * server answers `awaiting_signature`) this uses the deferred
   * pending-bundle / sign-callback flow:
   *
   * 1. `POST /mcp tools/call mnemonic_sign_memory` returns a `correlation_id`.
   * 2. `GET /api/pending/{correlation_id}` fetches the canonical-CBOR bytes
   *    (verbatim — never re-encoded in JS).
   * 3. `coseSignPayload(cbor, keypair)` wraps in COSE_Sign1 locally.
   * 4. `POST /api/sign-callback` (no Bearer JWT — capability auth via
   *    `correlation_id` + `signer_pubkey` + signature chain) returns
   *    `attestation_id`.
   *
   * @param content - Non-empty UTF-8 string to sign.
   * @param opts    - Optional tags array (forwarded to the server verbatim)
   *                  and write `mode` (`local` / `participate`).
   * @returns The `attestation_id`, server-issued `signed_at`, the terminal
   *          `status` (`stored` / `signed` / `pending` / `anchored`), the
   *          applied `writeMode` when known, and any optional
   *          `arweave_tx` / `solana_tx` / `content_hash` echoes.
   * @throws `UserError` if `content` is empty, `mode` is invalid, or a
   *         signature is needed but no keypair is available (call
   *         {@link setKeypair} or {@link setKeypairProvider} first). Without
   *         `mode: "local"` this check runs before any request. Errors from
   *         the keypair provider propagate unchanged.
   * @throws `AuthError` on 401 / 403 from `/mcp` or the sign-callback.
   * @throws `ServerError` on 5xx, network failure, or malformed JSON.
   * @throws `IntegrityError` if the sign-callback omits `attestation_id`
   *         (defence-in-depth — the server re-verifies, but failing fast
   *         here gives a better error).
   */
  async signMemory(
    content: string,
    opts: SignMemoryOptions = {}
  ): Promise<SignMemoryResult> {
    if (typeof content !== "string" || content.length === 0) {
      throw new UserError("signMemory: content must be a non-empty string");
    }
    const mode = opts.mode;
    if (
      mode !== undefined &&
      mode !== "local" &&
      mode !== "anchored" &&
      mode !== "participate"
    ) {
      throw new UserError(
        `signMemory: mode must be "local" or "anchored", got ${JSON.stringify(
          mode
        )}`
      );
    }
    // Legacy (no mode) and participate writes always end in a client
    // signature — fail fast, before any network call, when no keypair
    // source exists. A local write needs no keypair.
    if (mode !== "local" && !this.keypairJson && !this.keypairProvider) {
      throw new UserError(
        "signMemory: no keypair bound — call setKeypair(keypair) or setKeypairProvider(fn) before signMemory"
      );
    }

    const args: Record<string, unknown> = { content };
    if (opts.tags && opts.tags.length > 0) args.tags = opts.tags;
    if (mode !== undefined) args.mode = mode;

    // 1. Open the write — the server either stores a hash-only row
    //    (`local`) or returns a correlation_id for the deferred sign.
    const openResp = await this.callTool("mnemonic_sign_memory", args);
    const open = isRecord(openResp) ? openResp : {};
    const correlationId =
      typeof open.correlation_id === "string" ? open.correlation_id : null;
    if (!correlationId) {
      if (
        typeof open.attestation_id === "string" &&
        open.attestation_id.length > 0 &&
        open.status !== "awaiting_signature"
      ) {
        return storedRowResult(open, open.attestation_id, mode);
      }
      throw new ServerError(
        `mnemonic_sign_memory did not return correlation_id or attestation_id; got ${redactJWT(
          JSON.stringify(open)
        )}`
      );
    }

    // A signature is needed from here on — resolve the keypair now (this
    // is the only place a lazy provider runs).
    const keypairJson = await this.resolveKeypairJson();

    // 2. Fetch the canonical-CBOR bundle for the correlation_id.
    const pendingUrl = `${this.baseUrl}/api/pending/${encodeURIComponent(
      correlationId
    )}`;
    const pendingRes = await safeFetch(this.fetchImpl, pendingUrl, {
      method: "GET",
      headers: { Accept: "application/cbor" },
    });
    if (pendingRes.status === 404 || pendingRes.status === 410) {
      throw new ServerError(
        `pending bundle not available (HTTP ${pendingRes.status})`,
        pendingRes.status
      );
    }
    if (!pendingRes.ok) {
      throw new ServerError(
        `failed to fetch pending bundle (HTTP ${pendingRes.status})`,
        pendingRes.status
      );
    }
    const cborBytes = new Uint8Array(await pendingRes.arrayBuffer());
    if (cborBytes.length === 0) {
      throw new ServerError("pending bundle is empty");
    }

    // 2b. SEALED_V1 integrity guard: if the pending bundle is a sealed
    //     memory artifact, decrypt it with our X25519 key (derived from the
    //     Ed25519 signing keypair) and verify the plaintext matches `content`.
    //     This prevents signing a bundle whose encrypted content differs from
    //     what we submitted.
    await verifySealedBundleIfNeeded(cborBytes, keypairJson, content);

    // 3. COSE-sign the bytes (verbatim — DO NOT re-encode in JS, the server
    //    built these bytes and any drift breaks content_integrity).
    const cose = await coseSignPayload(cborBytes, keypairJson);

    // 4. POST /api/sign-callback (NO Bearer JWT — capability auth via
    //    correlation_id + signature chain, identical to the webapp flow).
    const callbackUrl = `${this.baseUrl}/api/sign-callback`;
    const callbackRes = await safeFetch(this.fetchImpl, callbackUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        correlation_id: correlationId,
        cose_signed_bytes: bytesToBase64(cose),
        signer_pubkey: this.signer.pubkey,
      }),
    });
    if (callbackRes.status === 410) {
      throw new ServerError(
        "pending bundle expired or already consumed",
        callbackRes.status
      );
    }
    if (callbackRes.status === 401 || callbackRes.status === 403) {
      throw new AuthError(`sign-callback rejected: HTTP ${callbackRes.status}`);
    }
    if (!callbackRes.ok) {
      const detail = await readBodySafely(callbackRes);
      throw new ServerError(
        `sign-callback failed: HTTP ${callbackRes.status} ${detail}`,
        callbackRes.status
      );
    }

    const cbBody = (await callbackRes.json().catch(() => ({}))) as Record<
      string,
      unknown
    >;
    const attestationId =
      typeof cbBody.attestation_id === "string" ? cbBody.attestation_id : null;
    if (!attestationId) {
      throw new IntegrityError("sign-callback did not return attestation_id");
    }
    return {
      attestationId,
      signedAt:
        typeof cbBody.signed_at === "string"
          ? cbBody.signed_at
          : new Date().toISOString(),
      status:
        cbBody.status === "anchored"
          ? "anchored"
          : cbBody.status === "pending"
          ? "pending"
          : "signed",
      ...writeModeField(cbBody.write_mode, mode),
      ...(typeof cbBody.content_hash === "string"
        ? { contentHash: cbBody.content_hash }
        : {}),
      ...(typeof cbBody.arweave_tx === "string"
        ? { arweaveTx: cbBody.arweave_tx }
        : {}),
      ...(typeof cbBody.solana_tx === "string"
        ? { solanaTx: cbBody.solana_tx }
        : {}),
    };
  }

  /**
   * Call the server's `mnemonic_recall` MCP tool.
   *
   * @param query - Query string used for semantic search.
   * @param opts  - Optional `topK` (default server-side) and `tags` filter.
   * @returns A `RecallResult` with normalised `hits` and a `total` count.
   * @throws `AuthError` on 401/403, `ServerError` on 5xx / network failure.
   */
  async recall(
    query: string,
    opts: { topK?: number; tags?: string[] } = {}
  ): Promise<RecallResult> {
    const args: Record<string, unknown> = { query };
    if (typeof opts.topK === "number") args.limit = opts.topK;
    if (opts.tags && opts.tags.length > 0) args.tags = opts.tags;
    const result = await this.callTool("mnemonic_recall", args);
    const raw = isRecord(result) ? result : {};
    const hitsRaw = Array.isArray(raw.hits)
      ? raw.hits
      : Array.isArray(raw.results)
      ? raw.results
      : [];
    const hits: RecallHit[] = hitsRaw.filter(isRecord).map((h) => ({
      ...h,
      attestationId:
        typeof h.attestation_id === "string" ? h.attestation_id : "",
      content: typeof h.content === "string" ? h.content : "",
      similarity: typeof h.similarity === "number" ? h.similarity : 0,
      ...(typeof h.signed_at === "string" ? { signedAt: h.signed_at } : typeof h.created_at === "string" ? { signedAt: h.created_at } : {}),
      ...(Array.isArray(h.tags)
        ? { tags: h.tags.filter((t) => typeof t === "string") as string[] }
        : {}),
    }));
    const total = typeof raw.total_attestations === "number" ? raw.total_attestations : typeof raw.total === "number" ? raw.total : hits.length;
    return { hits, total, evidence: raw };
  }

  /**
   * Call the server's `mnemonic_verify` MCP tool.
   *
   * @param attestationId - Non-empty attestation identifier returned by a
   *                        prior `signMemory` call.
   * @returns A discriminated union: `verified` (with `signer` + optional
   *          `arweave_tx` / `solana_tx`), `tampered` (with `signer` +
   *          `reason`), or `not_found`.
   * @throws `UserError` if `attestationId` is empty / non-string.
   * @throws `AuthError` on 401/403, `ServerError` on 5xx / network failure.
   */
  async verify(attestationId: string): Promise<VerifyResult> {
    if (!attestationId || typeof attestationId !== "string") {
      throw new UserError("verify: attestationId must be a non-empty string");
    }
    const result = await this.callTool("mnemonic_verify", {
      attestation_id: attestationId,
    });
    const raw = isRecord(result) ? result : {};
    const status = typeof raw.status === "string" ? raw.status : "not_found";
    if (status === "verified") {
      const out: VerifyResult = {
        status: "verified",
        signer: typeof raw.signer === "string" ? raw.signer : "",
      };
      if (typeof raw.arweave_tx === "string") out.arweaveTx = raw.arweave_tx;
      if (typeof raw.solana_tx === "string") out.solanaTx = raw.solana_tx;
      return out;
    }
    if (status === "tampered") {
      return {
        status: "tampered",
        signer: typeof raw.signer === "string" ? raw.signer : "",
        reason: typeof raw.reason === "string" ? raw.reason : "unknown",
      };
    }
    return { status: "not_found" };
  }

  /**
   * Call the server's `mnemonic_prove_identity` MCP tool — the **server**
   * signs the supplied challenge with its own keypair. Not used by the
   * CLI (Decision 14 — `mnemonic prove` signs locally instead) but
   * exposed for SDK consumers.
   *
   * @param challenge - Non-empty hex / base58 challenge string.
   * @returns `{pubkey, challenge, signature}` plus optional `did` and the
   *          raw server payload.
   * @throws `UserError` if `challenge` is empty / non-string.
   * @throws `AuthError` on 401/403, `ServerError` on 5xx / network failure.
   */
  async proveIdentity(challenge: string): Promise<ProveResult> {
    if (!challenge || typeof challenge !== "string") {
      throw new UserError(
        "proveIdentity: challenge must be a non-empty string"
      );
    }
    const result = await this.callTool("mnemonic_prove_identity", {
      challenge,
    });
    const raw = isRecord(result) ? result : {};
    const out: ProveResult = {
      pubkey: typeof raw.public_key === "string" ? raw.public_key :
        typeof raw.pubkey === "string" ? raw.pubkey : "",
      challenge: typeof raw.challenge === "string" ? raw.challenge : challenge,
      signature: typeof raw.signature === "string" ? raw.signature : "",
      raw,
    };
    if (typeof raw.did_sol === "string") out.did = raw.did_sol;
    else if (typeof raw.did === "string") out.did = raw.did;
    return out;
  }

  // ------------------------------------------------------------------------
  // Sealed-memory methods (T7)
  // ------------------------------------------------------------------------

  /**
   * Encrypt and sign a memory locally, then optionally deliver its signed bytes.
   *
   * The content is encrypted client-side with a fresh ephemeral key before
   * leaving this device. The server never sees the plaintext.
   *
   * @param content - Non-empty plaintext to seal.
   * @param opts    - `mode` (`"anchor"` | `"store"`), optional `tags`, and
   *                  optional custom `embedder`.
   * @returns `{memoryHash}` — blake3 hex of the sealed outer CBOR (the
   *          canonical on-chain identifier).
   * @throws `UserError` if content is empty or the keypair is not bound.
   * @throws `AuthError` on 401/403, `ServerError` on 5xx / network failure.
   */
  async sealMemory(
    content: string,
    opts: SealMemoryOptions
  ): Promise<SealMemoryResult> {
    if (typeof content !== "string" || content.length === 0) {
      throw new UserError("sealMemory: content must be a non-empty string");
    }
    if (opts.mode !== "store" && opts.mode !== "anchor") throw new UserError("sealMemory: mode must be store or anchor");
    const keypairJson = await this.resolveKeypairJson();
    const wasm = await loadWasm();

    // Derive author's Ed25519 public key bytes (32 bytes) from keypair.
    const ed25519Pub = new Uint8Array(keypairJson.secret.slice(32, 64));

    // Preserve the existing seal_memory JSON contract; bind both inner and outer identity.
    const artifactId = `art:${randomHex(16)}`;
    const now = new Date().toISOString();
    const innerObj: Record<string, unknown> = {
      artifact_id: artifactId, type: "memory", schema_version: 1,
      producer: `did:sol:${keypairJson.pubkey_base58}`, created_at: now, content,
    };
    if (opts.tags && opts.tags.length > 0) innerObj.tags = opts.tags;
    const innerJson = new TextEncoder().encode(JSON.stringify(innerObj));

    // Seal client-side via WASM.
    if (!wasm.seal_memory) {
      throw new ServerError(
        "sealMemory: WASM seal_memory binding not available"
      );
    }
    const sealResult = wasm.seal_memory(
      innerJson,
      ed25519Pub,
      artifactId,
      `did:sol:${keypairJson.pubkey_base58}`,
      now
    ) as { outer_cbor: Uint8Array; content_hash: Uint8Array } | null;

    // serde-wasm-bindgen serializes Vec<u8> as JS arrays in released builds.
    if (sealResult && Array.isArray(sealResult.outer_cbor)) sealResult.outer_cbor = Uint8Array.from(sealResult.outer_cbor);
    if (sealResult && Array.isArray(sealResult.content_hash)) sealResult.content_hash = Uint8Array.from(sealResult.content_hash);
    if (
      !sealResult ||
      !(sealResult.outer_cbor instanceof Uint8Array) ||
      !(sealResult.content_hash instanceof Uint8Array)
    ) {
      throw new ServerError("sealMemory: WASM seal_memory returned unexpected shape");
    }

    const signedBytes = await coseSignPayload(sealResult.outer_cbor, keypairJson);
    const memoryHash = bytesToHex(sealResult.content_hash);
    this.sealedLocal.set(memoryHash, {outerCbor: sealResult.outer_cbor.slice(), signedBytes: signedBytes.slice()});
    // "store" is now genuinely local: callers persist returned bytes for restart recovery.
    if (opts.mode === "store") return {memoryHash, signedBytes, outerCbor: sealResult.outer_cbor.slice()};
    const receipt = await this.ingestPreparedMemory(signedBytes);
    if (receipt.content_hash !== memoryHash || receipt.author !== keypairJson.pubkey_base58) {
      throw new IntegrityError("sealMemory: receipt identity mismatch");
    }
    return {memoryHash, signedBytes, outerCbor: sealResult.outer_cbor.slice(), locator: receipt.locator as string, receipt};
  }

  /** Prepare public MEMORY_V1 locally, without submitting or disclosing it. */
  async preparePublicMemory(content: string, opts: {tags?: string[]} = {}): Promise<{signedBytes: Uint8Array; contentHash: string}> {
    if (typeof content !== "string" || !content) throw new UserError("preparePublicMemory: content required");
    const kp=await this.resolveKeypairJson();
    const wasm=await loadWasm();
    if (!wasm.to_canonical_cbor_bytes || !wasm.blake3_hash_hex) throw new ServerError("canonical memory WASM bindings unavailable");
    const payload=wasm.to_canonical_cbor_bytes({artifact_id:`art:${randomHex(16)}`,type:"memory",schema_version:1,content,producer:`did:sol:${kp.pubkey_base58}`,created_at:new Date().toISOString(),visibility:"public",...(opts.tags?{tags:opts.tags}:{})});
    return {signedBytes:await coseSignPayload(payload,kp),contentHash:wasm.blake3_hash_hex(payload)};
  }

  /** Obtain original prepared bytes after a failed delivery, for caller-owned backup/retry. */
  preparedSealedMemories(): Array<{memoryHash: string; outerCbor: Uint8Array; signedBytes: Uint8Array}> {
    return [...this.sealedLocal].map(([memoryHash, row]) => ({memoryHash, outerCbor: row.outerCbor.slice(), signedBytes: row.signedBytes.slice()}));
  }

  /** Deliver original signed bytes. Public MEMORY_V1 requires explicit disclosure consent.
   * Retry the SAME bytes after payment/delivery failures; do not reseal.
   */
  async ingestPreparedMemory(signedBytes: Uint8Array, opts: {publicConsent?: boolean; operationId?: string; paymentHeaders?: Record<string,string>} = {}): Promise<Record<string, unknown>> {
    if (!(signedBytes instanceof Uint8Array) || signedBytes.length === 0 || signedBytes.length > 1048576) throw new UserError("invalid signed artifact size");
    const headers: Record<string,string> = {...opts.paymentHeaders, "Content-Type":"application/cbor", "x-mnemonic-mode":"anchored"};
    if (this.jwt) headers.Authorization = `Bearer ${this.jwt}`;
    if (opts.publicConsent) headers["x-mnemonic-public-consent"] = "true";
    if (opts.operationId) headers["x-mnemonic-operation-id"] = opts.operationId;
    const response = await safeFetch(this.fetchImpl, `${this.baseUrl}/api/ingest-artifact`, {method:"POST", headers, body: signedBytes as BodyInit});
    if (response.status === 401 || response.status === 403) throw new AuthError(`ingestion unauthorized (HTTP ${response.status})`);
    if (!response.ok) {
      const detail=await readBodySafely(response);
      let structured: unknown; try {structured=JSON.parse(detail);} catch {structured={error:detail};}
      throw new ServerError(`ingestion failed (HTTP ${response.status}); retain original signed bytes`,response.status,{response:structured,paymentRequired:response.headers.get("payment-required"),operationId:opts.operationId});
    }
    const receipt = await response.json() as Record<string,unknown>;
    if (receipt.delivery_status !== "verified" || typeof receipt.locator !== "string" || !/^ar:\/\/[A-Za-z0-9_-]{43}$/.test(receipt.locator)) throw new IntegrityError("invalid delivery receipt");
    const fetched = await boundedBody(await this._a2aExternal(`${this._a2aGateway()}/${receipt.locator.slice(5)}`));
    if (fetched.length !== signedBytes.length || fetched.some((b,i)=>b!==signedBytes[i])) throw new IntegrityError("external delivery bytes mismatch");
    return receipt;
  }

  /**
   * Open a session-cached sealed payload by hash, or restored bytes using the
   * identity's X25519 key (derived from the bound Ed25519 keypair).
   *
   * @param hashOrBytes - Hex content hash string or raw bytes of the outer
   *                      CBOR (if the caller already has them).
   * @param opts.expectedHash - Optional BLAKE3 hash of the original outer bytes;
   *                            checked before key access and decryption.
   * @returns The plaintext content and the raw inner JSON bytes.
   * @throws `UserError` if no keypair is bound.
   * @throws `IntegrityError` if decryption fails (wrong key or tampered).
   * @throws `AuthError` / `ServerError` on HTTP errors.
   */
  async openMemory(hashOrBytes: string | Uint8Array, opts: {expectedHash?: string} = {}): Promise<OpenMemoryResult> {
    let outerCbor: Uint8Array;
    if (hashOrBytes instanceof Uint8Array) {
      outerCbor = hashOrBytes;
    } else {
      const local = this.sealedLocal.get(hashOrBytes);
      if (!local) throw new UserError("openMemory: restore original outer CBOR from your backup; hosted blob storage is retired");
      outerCbor = local.outerCbor.slice();
    }

    const wasm = await loadWasm();
    if (opts.expectedHash !== undefined) {
      if (!/^[a-f0-9]{64}$/.test(opts.expectedHash)) throw new UserError("openMemory: invalid expected hash");
      if (!wasm.blake3_hash_hex) throw new ServerError("openMemory: hash verification binding unavailable");
      if (wasm.blake3_hash_hex(outerCbor) !== opts.expectedHash) throw new IntegrityError("openMemory: ciphertext hash mismatch");
    }
    const keypairJson = await this.resolveKeypairJson();
    if (!wasm.open_memory) {
      throw new ServerError("openMemory: WASM open_memory binding not available");
    }

    // Derive X25519 secret from Ed25519 keypair seed (first 32 bytes).
    if (!wasm.x25519_secret_from_seed) throw new ServerError("X25519 derivation binding unavailable");
    const ed25519Secret = wasm.x25519_secret_from_seed(new Uint8Array(keypairJson.secret.slice(0,32)));
    let innerBytes: Uint8Array;
    try {
      innerBytes = wasm.open_memory(outerCbor, ed25519Secret);
    } catch (e) {
      throw new IntegrityError(
        `openMemory: decryption failed — ${describeError(e)}`,
        e
      );
    } finally { ed25519Secret.fill(0); }

    const innerText = new TextDecoder().decode(innerBytes);
    let content = innerText;
    try {
      const parsed = JSON.parse(innerText) as Record<string, unknown>;
      if (typeof parsed.content === "string") content = parsed.content;
    } catch {
      // Not JSON; use raw text.
    }
    return { content, innerJson: innerBytes };
  }

  /**
   * Grant access to a sealed memory.
   *
   * - For a targeted reader (`{kid, x25519Pub}`): wraps `K` for the reader's
   *   X25519 key and returns the GRANT_V1 CBOR bytes.
   * - For `"link"`: encodes `K` as a URL fragment and returns a shareable URL
   *   with a `#k=<base64url>` fragment.
   *
   * @param memoryHash   - Hex content hash identifying the sealed memory.
   * @param targetOrHash - Targeted reader descriptor, or `"link"` for an
   *                        anonymous bearer link.
   * @returns `{type: "grant", grantCbor}` or `{type: "link", url}`.
   * @throws `UserError` if no keypair is bound.
   * @throws `AuthError` / `ServerError` on HTTP errors.
   */
  async share(
    memoryHash: string,
    target: ShareTarget
  ): Promise<ShareResult> {
    throw new UserError("share: hosted grant creation is retired; distribute signed grants client-side or use sealed A2A recipient grants");
  }

  /**
   * Parse a `#k=<base64url>` fragment from a share URL, fetch the sealed blob
   * by hash (extracted from the URL path), and decrypt using the bearer key.
   *
   * @param url - Share URL with a `#k=<base64url-nopad>` fragment.
   * @returns The decrypted memory content.
   * @throws `UserError` if the URL has no `#k=` fragment or no hash in the path.
   * @throws `IntegrityError` if decryption fails.
   * @throws `ServerError` on fetch errors.
   */
  async importLink(url: string): Promise<OpenMemoryResult> {
    const hashIdx = url.indexOf("#");
    if (hashIdx === -1) {
      throw new UserError("importLink: URL has no fragment (#k=...)");
    }
    const fragment = url.slice(hashIdx + 1);
    if (!fragment.startsWith("k=")) {
      throw new UserError(
        `importLink: expected fragment starting with k=, got ${fragment.slice(0, 20)}`
      );
    }

    // Parse hash from URL path (last path segment before #).
    const pathPart = url.slice(0, hashIdx);
    const pathSegments = pathPart.split("/").filter(Boolean);
    const memoryHash = pathSegments[pathSegments.length - 1] ?? "";
    if (!memoryHash) {
      throw new UserError("importLink: could not extract memory hash from URL path");
    }

    const wasm = await loadWasm();
    if (!wasm.parse_link_fragment || !wasm.open_with_key) {
      throw new ServerError("importLink: WASM sealed bindings not available");
    }

    // Parse K from fragment.
    let kBytes: Uint8Array;
    try {
      kBytes = wasm.parse_link_fragment(fragment);
    } catch (e) {
      throw new UserError(`importLink: invalid fragment: ${describeError(e)}`, e);
    }

    // Fetch outer CBOR.
    const fetchUrl = `${this.baseUrl}/api/sealed/${encodeURIComponent(memoryHash)}`;
    const fetchHeaders: Record<string, string> = { Accept: "application/cbor" };
    if (this.jwt) fetchHeaders.Authorization = `Bearer ${this.jwt}`;
    const res = await safeFetch(this.fetchImpl, fetchUrl, {
      method: "GET",
      headers: fetchHeaders,
    });
    if (res.status === 404) {
      throw new ServerError(`importLink: memory not found (${memoryHash})`, 404);
    }
    if (!res.ok) {
      const detail = await readBodySafely(res);
      throw new ServerError(
        `importLink: failed to fetch memory (HTTP ${res.status}) ${detail}`,
        res.status
      );
    }
    const outerCbor = new Uint8Array(await res.arrayBuffer());

    // Decrypt with K.
    let innerBytes: Uint8Array;
    try {
      innerBytes = wasm.open_with_key(outerCbor, kBytes);
    } catch (e) {
      throw new IntegrityError(
        `importLink: decryption failed — ${describeError(e)}`,
        e
      );
    } finally { kBytes.fill(0); }

    const innerText = new TextDecoder().decode(innerBytes);
    let content = innerText;
    try {
      const parsed = JSON.parse(innerText) as Record<string, unknown>;
      if (typeof parsed.content === "string") content = parsed.content;
    } catch {
      // Not JSON; use raw text.
    }
    return { content, innerJson: innerBytes };
  }

  /**
   * List all grants the caller has created (or received, by reader DID).
   *
   * @returns Array of grant metadata entries.
   * @throws `AuthError` / `ServerError` on HTTP errors.
   */
  async listGrants(): Promise<GrantEntry[]> {
    const url = `${this.baseUrl}/api/grants?reader=${encodeURIComponent(
      this.signer.pubkey
    )}`;
    const headers: Record<string, string> = { Accept: "application/json" };
    if (this.jwt) headers.Authorization = `Bearer ${this.jwt}`;
    const res = await safeFetch(this.fetchImpl, url, {
      method: "GET",
      headers,
    });
    if (res.status === 401 || res.status === 403) {
      throw new AuthError(`listGrants: unauthorized (HTTP ${res.status})`);
    }
    if (!res.ok) {
      const detail = await readBodySafely(res);
      throw new ServerError(
        `listGrants: failed (HTTP ${res.status}) ${detail}`,
        res.status
      );
    }
    const body = (await res.json().catch(() => ({ grants: [] }))) as Record<
      string,
      unknown
    >;
    const raw = Array.isArray(body.grants) ? body.grants : [];
    return raw.filter(isRecord).map((g) => {
      const entry: { grantId: string; memoryHash: string; createdAt: string; reader?: string } = {
        grantId: typeof g.grant_id === "string" ? g.grant_id : "",
        memoryHash: typeof g.memory_hash === "string" ? g.memory_hash : "",
        createdAt:
          typeof g.created_at === "string" ? g.created_at : new Date().toISOString(),
      };
      if (typeof g.reader === "string") entry.reader = g.reader;
      return entry;
    });
  }

  /** Rank this client's sealed cache using an explicitly supplied local embedder.
   * No plaintext query, memory, embedding or recall key is sent to the operator.
   */
  async recallSealed(query: string, opts: RecallSealedOptions = {}): Promise<SealedHit[]> {
    const topK=opts.topK??10;
    if (!Number.isSafeInteger(topK) || topK < 1) throw new UserError("recallSealed: topK must be positive");
    if (!opts.embedder) throw new UserError("recallSealed: supply a local embedder; hosted embedding is not a private recall path");
    const queryVector=await opts.embedder.embed(query);
    const hits: SealedHit[]=[];
    for (const [memoryHash,row] of this.sealedLocal) {
      const opened=await this.openMemory(row.outerCbor);
      const embedding=await opts.embedder.embed(opened.content);
      hits.push({memoryHash,similarity:cosineSimilarity(queryVector,embedding)});
    }
    return hits.sort((a,b)=>b.similarity-a.similarity).slice(0,topK);
  }

  // ------------------------------------------------------------------------
  // A2A attestation methods (Task 7)
  // ------------------------------------------------------------------------

  /**
   * Attest an A2A Task object under `contextId`. Returns the opaque
   * `attestation_id` for the new attestation.
   *
   * @param task      - The A2A Task object (must have a non-empty `id`).
   * @param contextId - Context that groups related A2A attestations.
   * @param opts      - Optional `prevId` linking to a prior attestation.
   * @throws `UserError`  if `task.id` or `contextId` is missing.
   * @throws `ServerError` if the server does not return `attestation_id`.
   * @throws `AuthError`  on 401/403.
   */
  attestA2ATask(
    task: A2ATask,
    contextId: string,
    opts?: AttestA2AOptions
  ): Promise<AttestationId> {
    return attestA2ATask.call(this, task, contextId, opts);
  }

  /**
   * Attest an A2A Message object under `contextId`.
   *
   * @param msg       - The A2A Message object (must have a non-empty `messageId`).
   * @param contextId - Context that groups related A2A attestations.
   * @param opts      - Optional `prevId`.
   */
  attestA2AMessage(
    msg: A2AMessage,
    contextId: string,
    opts?: AttestA2AOptions
  ): Promise<AttestationId> {
    return attestA2AMessage.call(this, msg, contextId, opts);
  }

  /**
   * Attest an A2A Artifact object under `contextId`.
   *
   * @param art       - The A2A Artifact object (must have a non-empty `artifactId`).
   * @param contextId - Context that groups related A2A attestations.
   * @param opts      - Optional `prevId`.
   */
  attestA2AArtifact(
    art: A2AArtifact,
    contextId: string,
    opts?: AttestA2AOptions
  ): Promise<AttestationId> {
    return attestA2AArtifact.call(this, art, contextId, opts);
  }

  /**
   * Recall all attestations anchored under a given A2A context.
   *
   * @param contextId - The context identifier to query.
   * @param opts      - Optional `limit` and `kind` filter.
   * @returns An array of `Attestation` records (tasks, messages, artifacts).
   */
  recallA2AContext(
    contextId: string,
    opts?: RecallA2AContextOptions
  ): Promise<Attestation[]> {
    return recallA2AContext.call(this, contextId, opts);
  }

  /** Verify the expected author and decrypt recalled bytes in this client. */
  openA2AAttestation(attestation: Attestation, expectedAuthor: string, encryptionSecret?: Uint8Array): Promise<OpenedA2A> {
    return openA2AAttestation.call(this,attestation,expectedAuthor,encryptionSecret);
  }

  /** @internal */
  async _resolveA2AKeypair(): Promise<KeypairJson> {
    const kp = await this.resolveKeypairJson();
    return {secret:[...kp.secret],pubkey_base58:kp.pubkey_base58};
  }

  /** Resume a retained A2A artifact without resigning/resealing it. */
  async retryA2ADelivery(attestationId: string, opts: {operationId?: string; paymentHeaders?: Record<string,string>; prevLocator?: string} = {}): Promise<Attestation> {
    const rows=await this.a2aIndex.list();
    const retained=rows.find(r=>r.attestationId===attestationId);
    if (!retained?.coseEnvelopeHex || !retained.signerPubkey) throw new UserError("retryA2ADelivery: original signed bytes are unavailable");
    const row=await decodeAttestation(hexToBytes(retained.coseEnvelopeHex),retained.signerPubkey);
    const parentLocator=opts.prevLocator??rows.find(r=>r.attestationId===row.prevId)?.locator;
    if (row.prevId && !parentLocator) throw new UserError("ParentLocatorRequired");
    const headers={...opts.paymentHeaders,...(opts.operationId?{"x-mnemonic-operation-id":opts.operationId}:{})};
    const result=await this.callTool("mnemonic_attest_a2a",{kind:row.kind,context_id:row.contextId,signed:row.coseEnvelopeHex,sealed:row.sealed,mode:"anchored",...(row.prevId?{prev_id:row.prevId,prev_locator:parentLocator}:{})},headers);
    if (!isRecord(result) || result.attestation_id!==row.attestationId || typeof result.locator!=="string") throw new IntegrityError("A2A retry receipt identity mismatch");
    const delivered=await fetchAttestation.call(this,result.locator,row.signerPubkey!);
    if (delivered.coseEnvelopeHex!==row.coseEnvelopeHex) throw new IntegrityError("A2A retry delivery mismatch");
    await this.a2aIndex.put(delivered);return delivered;
  }

  /**
   * Internal bridge so A2A mixin functions can call `callTool` without
   * exposing it on the public surface. Named `_callToolA2A` to signal that
   * it is for A2A mixin use only.
   *
   * @internal
   */
  _callToolA2A(
    name: string,
    args: Record<string, unknown>,
    extraHeaders: Record<string,string> = {}
  ): Promise<unknown> {
    return this.callTool(name, args, extraHeaders);
  }

  // ------------------------------------------------------------------------
  // Internal: lazy keypair + access-token refresh
  // ------------------------------------------------------------------------

  /**
   * Return the keypair JSON for a COSE signature: the one bound with
   * `setKeypair`, else the (memoised) provider result. Checks that it
   * matches `signer.pubkey`, because the sign-callback sends that pubkey.
   */
  private async resolveKeypairJson(): Promise<KeypairJson> {
    let json: KeypairJson;
    if (this.keypairJson) {
      json = this.keypairJson;
    } else if (this.keypairProvider) {
      if (!this.providedKeypair) {
        const provider = this.keypairProvider;
        const p = Promise.resolve()
          .then(() => provider())
          .then((kp) => kp.toJSON());
        // Forget a failed attempt so a later call can retry.
        p.catch(() => {
          if (this.providedKeypair === p) this.providedKeypair = null;
        });
        this.providedKeypair = p;
      }
      json = await this.providedKeypair;
    } else {
      throw new UserError(
        "signMemory: the server asked for a client signature, but no keypair is bound — call setKeypair(keypair) or setKeypairProvider(fn)"
      );
    }
    if (json.pubkey_base58 !== this.signer.pubkey) {
      throw new UserError(
        `signMemory: keypair pubkey ${json.pubkey_base58} does not match signer pubkey ${this.signer.pubkey}`
      );
    }
    return json;
  }

  /** True when the bound JWT expires within `REFRESH_SKEW_MS`. */
  private jwtNeedsRefresh(): boolean {
    if (!this.jwt) return true;
    const exp = readJwtExp(this.jwt);
    if (exp === undefined) return false; // opaque token: let the server decide
    return exp * 1000 - Date.now() <= REFRESH_SKEW_MS;
  }

  /** Run the refresher once, shared by concurrent callers. */
  private refreshJwt(refresher: TokenRefresher): Promise<string | undefined> {
    if (!this.refreshInFlight) {
      const p = (async () => {
        try {
          const fresh = await refresher();
          if (typeof fresh === "string" && fresh.length > 0) this.jwt = fresh;
          return fresh;
        } finally {
          this.refreshInFlight = null;
        }
      })();
      this.refreshInFlight = p;
    }
    return this.refreshInFlight;
  }

  /**
   * Tool call with access-token refresh. Refreshes first when the JWT is
   * missing or about to expire, and retries once after a 401 / 403 if the
   * refresher produced a different token. Without a refresher this is a
   * plain single call.
   */
  private async callTool(
    name: string,
    args: Record<string, unknown>,
    extraHeaders: Record<string,string> = {}
  ): Promise<unknown> {
    const refresher = this.tokenRefresher;
    if (!refresher) return this.callToolOnce(name, args, extraHeaders);
    if (this.jwtNeedsRefresh()) await this.refreshJwt(refresher);
    try {
      return await this.callToolOnce(name, args, extraHeaders);
    } catch (e) {
      if (!(e instanceof AuthError)) throw e;
      const before = this.jwt;
      const fresh = await this.refreshJwt(refresher);
      if (!fresh || fresh === before) throw e;
      return this.callToolOnce(name, args, extraHeaders);
    }
  }

  // ------------------------------------------------------------------------
  // Internal: JSON-RPC over HTTP to /mcp
  // ------------------------------------------------------------------------

  /**
   * Single MCP tool call. Wraps the `tools/call` JSON-RPC envelope, attaches
   * the Bearer JWT, normalizes the wide range of server response shapes
   * (some tools return their result in `result.content[0].text` JSON-encoded,
   * others return it in `result` directly).
   */
  private async callToolOnce(
    name: string,
    args: Record<string, unknown>,
    extraHeaders: Record<string,string> = {}
  ): Promise<unknown> {
    const url = `${this.baseUrl}/mcp`;
    const body = {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name, arguments: args },
    };

    const headers: Record<string, string> = {
      ...extraHeaders,
      "Content-Type": "application/json",
      Accept: "application/json",
    };
    if (this.jwt) headers.Authorization = `Bearer ${this.jwt}`;

    const res = await safeFetch(this.fetchImpl, url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });

    if (res.status === 401 || res.status === 403) {
      const detail = await readBodySafely(res);
      throw new AuthError(
        `${name} unauthorized (HTTP ${res.status}) ${detail}`
      );
    }
    if (!res.ok) {
      const detail = await readBodySafely(res);
      throw new ServerError(
        `${name} failed: HTTP ${res.status} ${detail}`,
        res.status,
        {response: (() => {try {return JSON.parse(detail);} catch {return {error:detail};}})(), paymentRequired:res.headers.get("payment-required")}
      );
    }

    let parsed: unknown;
    try {
      parsed = await res.json();
    } catch (e) {
      throw new ServerError(`${name}: malformed JSON response`, res.status, e);
    }

    if (!isRecord(parsed)) {
      throw new ServerError(`${name}: response was not an object`);
    }
    if (isRecord(parsed.error)) {
      const err = parsed.error;
      const msg =
        typeof err.message === "string" ? err.message : `${name} error`;
      // Map JSON-RPC error code 401/403 hints to AuthError.
      if (
        typeof err.code === "number" &&
        (err.code === 401 || err.code === 403)
      ) {
        throw new AuthError(msg);
      }
      throw new ServerError(msg, undefined, err);
    }

    return extractToolResult(parsed.result);
  }
}

// --------------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------------

/**
 * MCP tools/call results are wrapped in `{content: [{type:'text', text:'<JSON>'}]}`
 * in the canonical MCP wire format. Some servers return the parsed object
 * directly. Handle both.
 */
function extractToolResult(result: unknown): unknown {
  if (!isRecord(result)) return result;
  if (Array.isArray(result.content) && result.content.length > 0) {
    const first = result.content[0];
    if (isRecord(first) && typeof first.text === "string") {
      try {
        return JSON.parse(first.text);
      } catch {
        return first.text;
      }
    }
  }
  return result;
}

/** `{writeMode}` from the server's `write_mode`, else the requested mode. */
function writeModeField(
  raw: unknown,
  requested: WriteMode | undefined
): { writeMode?: WriteMode } {
  // Accept the legacy spelling so an older server response still parses.
  if (raw === "participate") return { writeMode: "anchored" };
  if (raw === "local" || raw === "anchored") return { writeMode: raw };
  return requested ? { writeMode: requested } : {};
}

/**
 * Project a stored-row response (a `local` write that the server stored
 * without a client signature) onto `SignMemoryResult`.
 */
function storedRowResult(
  open: Record<string, unknown>,
  attestationId: string,
  requested: WriteMode | undefined
): SignMemoryResult {
  const signedAt =
    typeof open.signed_at === "string"
      ? open.signed_at
      : typeof open.timestamp === "string"
      ? open.timestamp
      : typeof open.created_at === "string"
      ? open.created_at
      : new Date().toISOString();
  const status: SignMemoryResult["status"] =
    open.status === "anchored" ||
    open.status === "pending" ||
    open.status === "signed" ||
    open.status === "stored"
      ? open.status
      : typeof open.solana_tx === "string"
      ? "anchored"
      : "stored";
  return {
    attestationId,
    signedAt,
    status,
    ...writeModeField(open.write_mode, requested),
    ...(typeof open.content_hash === "string"
      ? { contentHash: open.content_hash }
      : {}),
    ...(typeof open.arweave_tx === "string"
      ? { arweaveTx: open.arweave_tx }
      : {}),
    ...(typeof open.solana_tx === "string" ? { solanaTx: open.solana_tx } : {}),
  };
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Wrap fetch with a typed-error rewrap so network failures surface as
 * `ServerError` rather than raw `TypeError: fetch failed`.
 */
async function safeFetch(
  f: typeof fetch,
  url: string,
  init: RequestInit
): Promise<Response> {
  try {
    return await f(url, init);
  } catch (e) {
    if (e instanceof MnemonicError) throw e;
    throw new ServerError(`network error: ${describeError(e)}`, undefined, e);
  }
}

async function readBodySafely(res: Response): Promise<string> {
  try {
    const txt = await res.text();
    // Redact BEFORE slicing — slicing first can cut a JWT mid-string and
    // leave the trailing portion (a partial header < 20 chars or the
    // signature segment) below the regex's {20,} threshold, which would
    // skip the redaction. See security-auditor round 1, finding #2.
    return redactJWT(txt).slice(0, 500);
  } catch {
    return "";
  }
}

function describeError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return JSON.stringify(e);
}

/** Encode bytes as standard-alphabet, padded base64. */
function bytesToBase64(bytes: Uint8Array): string {
  // Web-API path: use btoa over a binary string. atob/btoa are universally
  // available in Node 20+, Bun, Deno, and browsers.
  let s = "";
  // Chunk to avoid call-stack limits on very large arrays (>~64KB).
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    s += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(s);
}

/** Decode standard-alphabet base64 to bytes. */
function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** Encode bytes as lowercase hex. */
function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** Decode lowercase hex to bytes. Returns empty array on odd-length input. */
function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) return new Uint8Array(0);
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/** Generate N random bytes as a hex string. */
function randomHex(bytes: number): string {
  const arr = new Uint8Array(bytes);
  // Use Web Crypto if available (browser / Node 20+), else Math.random fallback.
  if (
    typeof globalThis !== "undefined" &&
    typeof (globalThis as { crypto?: { getRandomValues?: unknown } }).crypto
      ?.getRandomValues === "function"
  ) {
    (globalThis as unknown as { crypto: Crypto }).crypto.getRandomValues(arr);
  } else {
    for (let i = 0; i < arr.length; i++) arr[i] = Math.floor(Math.random() * 256);
  }
  return bytesToHex(arr);
}

/**
 * Compute cosine similarity between two float32 vectors.
 * Returns 0 when either vector has zero magnitude.
 */
function cosineSimilarity(a: Float32Array, b: Float32Array): number {
  const len = Math.min(a.length, b.length);
  let dot = 0;
  let magA = 0;
  let magB = 0;
  for (let i = 0; i < len; i++) {
    dot += (a[i]! * b[i]!);
    magA += a[i]! * a[i]!;
    magB += b[i]! * b[i]!;
  }
  const denom = Math.sqrt(magA) * Math.sqrt(magB);
  return denom === 0 ? 0 : dot / denom;
}

/**
 * Detect a SEALED_V1 pending bundle and verify its decrypted content matches
 * the input `content`. A SEALED_V1 bundle has `type: "sealed"` in its decoded
 * CBOR payload. We detect the type by trying to find the ASCII string "sealed"
 * near the start of the CBOR bytes (the type field is early in canonical CBOR).
 *
 * When detected:
 *   1. Use WASM `open_memory` with the Ed25519 seed's X25519 key to decrypt.
 *   2. Parse inner JSON and compare `content` field.
 *   3. Throw `IntegrityError` if the content does not match.
 *
 * When NOT detected (normal MEMORY_V1 bundle): returns without doing anything.
 */
async function verifySealedBundleIfNeeded(
  cborBytes: Uint8Array,
  keypairJson: KeypairJson,
  content: string
): Promise<void> {
  // Quick heuristic: search for `"type"` and `"sealed"` in the CBOR bytes.
  // Canonical CBOR encodes map keys and string values as UTF-8 text items.
  // The SEALED_V1 schema has a `"type": "sealed"` field in early position.
  // We scan the first 256 bytes for the byte sequence of "sealed".
  const sealedBytes = new TextEncoder().encode("sealed");
  const scanLen = Math.min(cborBytes.length, 512);
  let found = false;
  outer: for (let i = 0; i < scanLen - sealedBytes.length + 1; i++) {
    let match = true;
    for (let j = 0; j < sealedBytes.length; j++) {
      if (cborBytes[i + j] !== sealedBytes[j]) {
        match = false;
        break;
      }
    }
    if (match) {
      found = true;
      break outer;
    }
  }
  if (!found) return; // Not a SEALED_V1 bundle — nothing to verify.

  // Load WASM and open the sealed memory with the author's X25519 key.
  const wasm = await loadWasm();
  if (!wasm.open_memory) {
    // WASM sealed bindings not present — skip the integrity check rather than
    // fail hard, so non-sealed signers aren't broken. The server still verifies.
    return;
  }

  // Derive X25519 secret from Ed25519 seed (first 32 bytes of the keypair secret).
  if (!wasm.x25519_secret_from_seed) throw new ServerError("X25519 derivation binding unavailable");
  const ed25519Secret = wasm.x25519_secret_from_seed(new Uint8Array(keypairJson.secret.slice(0,32)));

  let innerBytes: Uint8Array;
  try {
    innerBytes = wasm.open_memory(cborBytes, ed25519Secret);
  } catch (e) {
    throw new IntegrityError(
      `signMemory: SEALED_V1 bundle could not be decrypted — ${describeError(e)}`,
      e
    );
  } finally { ed25519Secret.fill(0); }

  // Parse inner JSON and compare content.
  const innerText = new TextDecoder().decode(innerBytes);
  let innerContent = innerText;
  try {
    const parsed = JSON.parse(innerText) as Record<string, unknown>;
    if (typeof parsed.content === "string") innerContent = parsed.content;
  } catch {
    // Raw text — compare directly.
  }

  if (innerContent !== content) {
    throw new IntegrityError(
      "signMemory: SEALED_V1 bundle content does not match input — refusing to sign"
    );
  }
}
