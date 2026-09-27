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
// Exception — `mode: "local"`: the server stores a hash-only row and answers
// step 1 with `{attestation_id, content_hash, write_mode: "local"}` (no
// `correlation_id`). Steps 2-4 are skipped and no keypair is needed, so a
// lazy keypair source (`setKeypairProvider`) is never invoked.
//
// Access-token refresh (issue #33): with a `tokenRefresher`, `callTool`
// renews the JWT before it expires and retries once after a 401/403.

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
  KeypairProvider,
  MnemonicClientConfig,
  ProveResult,
  RecallHit,
  RecallResult,
  SignMemoryOptions,
  SignMemoryResult,
  SignerInterface,
  TokenRefresher,
  VerifyResult,
  WhoamiResult,
  WriteMode,
} from "./types.js";

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
    if (mode !== undefined && mode !== "local" && mode !== "participate") {
      throw new UserError(
        `signMemory: mode must be "local" or "participate", got ${JSON.stringify(
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
    if (typeof opts.topK === "number") args.top_k = opts.topK;
    if (opts.tags && opts.tags.length > 0) args.tags = opts.tags;
    const result = await this.callTool("mnemonic_recall", args);
    const raw = isRecord(result) ? result : {};
    const hitsRaw = Array.isArray(raw.hits)
      ? raw.hits
      : Array.isArray(raw.results)
      ? raw.results
      : [];
    const hits: RecallHit[] = hitsRaw.filter(isRecord).map((h) => ({
      attestationId:
        typeof h.attestation_id === "string" ? h.attestation_id : "",
      content: typeof h.content === "string" ? h.content : "",
      similarity: typeof h.similarity === "number" ? h.similarity : 0,
      ...(typeof h.signed_at === "string" ? { signedAt: h.signed_at } : {}),
      ...(Array.isArray(h.tags)
        ? { tags: h.tags.filter((t) => typeof t === "string") as string[] }
        : {}),
    }));
    const total = typeof raw.total === "number" ? raw.total : hits.length;
    return { hits, total };
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
      pubkey: typeof raw.pubkey === "string" ? raw.pubkey : "",
      challenge: typeof raw.challenge === "string" ? raw.challenge : challenge,
      signature: typeof raw.signature === "string" ? raw.signature : "",
      raw,
    };
    if (typeof raw.did === "string") out.did = raw.did;
    return out;
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
    args: Record<string, unknown>
  ): Promise<unknown> {
    const refresher = this.tokenRefresher;
    if (!refresher) return this.callToolOnce(name, args);
    if (this.jwtNeedsRefresh()) await this.refreshJwt(refresher);
    try {
      return await this.callToolOnce(name, args);
    } catch (e) {
      if (!(e instanceof AuthError)) throw e;
      const before = this.jwt;
      const fresh = await this.refreshJwt(refresher);
      if (!fresh || fresh === before) throw e;
      return this.callToolOnce(name, args);
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
    args: Record<string, unknown>
  ): Promise<unknown> {
    const url = `${this.baseUrl}/mcp`;
    const body = {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name, arguments: args },
    };

    const headers: Record<string, string> = {
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
        res.status
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
  if (raw === "local" || raw === "participate") return { writeMode: raw };
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
