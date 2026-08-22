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

import { coseSignPayload } from "./cose.js";
import {
  AuthError,
  IntegrityError,
  MnemonicError,
  PaymentRequiredError,
  ServerError,
  UserError,
  redactJWT,
} from "./errors.js";
import type { Keypair, KeypairJson } from "./keypair.js";
import type {
  MnemonicClientConfig,
  PaymentAuthorization,
  PaymentHandler,
  PaymentOperationBinding,
  PaymentQuote,
  PaymentReceipt,
  PreparedPaymentOperation,
  PaidOperationStatus,
  ProveResult,
  RecallHit,
  RecallResult,
  SignMemoryOptions,
  SignMemoryResult,
  SignedPaymentChallenge,
  SignerInterface,
  VerifyResult,
  WhoamiResult,
} from "./types.js";

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
  private readonly paymentHandler: PaymentHandler | undefined;
  /**
   * Optional keypair — needed only for `signMemory` (the COSE step needs
   * the JSON form, which `Signer.pubkey` can't provide). When the client
   * is built from an arbitrary `Signer`, `signMemory` will throw `UserError`.
   * `LocalSigner` consumers should call `setKeypair(keypair)` before
   * `signMemory`.
   */
  private keypairJson: KeypairJson | null = null;

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
    this.paymentHandler = config.paymentHandler;
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
   * Sign a memory. Always uses the deferred pending-bundle / sign-callback
   * flow:
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
   * @param opts    - Optional tags array (forwarded to the server verbatim).
   * @returns The `attestation_id`, server-issued `signed_at`, the terminal
   *          `status` (`signed` / `pending` / `anchored`), and any optional
   *          `arweave_tx` / `solana_tx` / `content_hash` echoes.
   * @throws `UserError` if `content` is empty or no keypair is bound (call
   *         {@link setKeypair} first).
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
    if (!this.keypairJson) {
      throw new UserError(
        "signMemory: no keypair bound — call setKeypair(keypair) before signMemory"
      );
    }

    const args: Record<string, unknown> = { content };
    if (opts.tags && opts.tags.length > 0) args.tags = opts.tags;
    if (opts.mode) args.mode = opts.mode;
    if (opts.visibility) args.visibility = opts.visibility;
    if (opts.checkpointType) args.checkpoint_type = opts.checkpointType;
    if (opts.workspace) args.workspace = opts.workspace;

    // 1. Open the deferred sign — server returns correlation_id.
    const openResp = await this.callTool("mnemonic_sign_memory", args);
    const open = isRecord(openResp) ? openResp : {};
    const correlationId =
      typeof open.correlation_id === "string" ? open.correlation_id : null;
    if (!correlationId) {
      throw new ServerError(
        `mnemonic_sign_memory did not return correlation_id; got ${redactJWT(
          JSON.stringify(open)
        )}`
      );
    }

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
    const cose = await coseSignPayload(cborBytes, this.keypairJson);

    // 4. POST /api/sign-callback (NO Bearer JWT — capability auth via
    //    correlation_id + signature chain, identical to the webapp flow).
    const callback = {
      correlation_id: correlationId,
      cose_signed_bytes: bytesToBase64(cose),
      signer_pubkey: this.signer.pubkey,
    };
    return this.submitSignedCallback(callback, opts.payment, true);
  }

  /** Continue a previously signed operation after an external wallet flow. */
  async resumePaidMemory(
    challenge: SignedPaymentChallenge,
    payment: PaymentAuthorization
  ): Promise<SignMemoryResult> {
    if (challenge.callback.signer_pubkey !== this.signer.pubkey) {
      throw new UserError("payment challenge belongs to a different signer");
    }
    if (challenge.operationId !== challenge.callback.correlation_id) {
      throw new IntegrityError("payment challenge operation binding is invalid");
    }
    if (challenge.bindingStatus === "provisional") {
      await this.preparePaidOperation(challenge.operationId, payment.payer_wallet);
    }
    return this.submitSignedCallback(challenge.callback, payment, false);
  }

  /**
   * Bind the connected payer wallet and return the final immutable operation
   * binding. Payment authorization must be signed against this response.
   */
  async preparePaidOperation(
    operationId: string,
    payerWallet: string
  ): Promise<PreparedPaymentOperation> {
    if (!operationId || !payerWallet) {
      throw new UserError("preparePaidOperation: operationId and payerWallet are required");
    }
    const response = await safeFetch(
      this.fetchImpl,
      `${this.baseUrl}/api/paid-operations/${encodeURIComponent(operationId)}/prepare`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: JSON.stringify({ payer_wallet: payerWallet }),
      }
    );
    if (!response.ok) {
      const detail = await readBodySafely(response);
      throw new ServerError(
        `payment preparation failed: HTTP ${response.status} ${detail}`,
        response.status
      );
    }
    const body = (await response.json().catch(() => ({}))) as Record<
      string,
      unknown
    >;
    if (
      body.status !== "payment_prepared" ||
      body.operation_id !== operationId ||
      body.binding_status !== "final" ||
      typeof body.binding_digest !== "string"
    ) {
      throw new ServerError("payment preparation response was malformed");
    }
    const binding = parsePaymentBinding(body.binding);
    if (
      binding.operation_id !== operationId ||
      binding.payer_wallet.toLowerCase() !== payerWallet.toLowerCase()
    ) {
      throw new IntegrityError("prepared payment binding does not match request");
    }
    return {
      operationId,
      binding,
      bindingDigest: body.binding_digest,
    };
  }

  /** Read durable payment/anchoring progress. This call is strictly read-only. */
  async paidOperationStatus(operationId: string): Promise<PaidOperationStatus> {
    if (!operationId || typeof operationId !== "string") {
      throw new UserError("paidOperationStatus: operationId is required");
    }
    const response = await safeFetch(
      this.fetchImpl,
      `${this.baseUrl}/api/paid-operations/${encodeURIComponent(operationId)}`,
      { method: "GET", headers: { Accept: "application/json" } }
    );
    if (response.status === 401 || response.status === 403) {
      throw new AuthError(`payment status rejected: HTTP ${response.status}`);
    }
    if (!response.ok) {
      const detail = await readBodySafely(response);
      throw new ServerError(
        `payment status failed: HTTP ${response.status} ${detail}`,
        response.status
      );
    }
    const body = (await response.json().catch(() => ({}))) as Record<
      string,
      unknown
    >;
    return parsePaidOperationStatus(body, operationId);
  }

  private async submitSignedCallback(
    callback: SignedPaymentChallenge["callback"],
    payment: PaymentAuthorization | undefined,
    allowPaymentHandler: boolean
  ): Promise<SignMemoryResult> {
    const callbackRes = await safeFetch(
      this.fetchImpl,
      `${this.baseUrl}/api/sign-callback`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          ...callback,
          ...(payment ? { payment } : {}),
        }),
      }
    );

    if (callbackRes.status === 402) {
      const raw = (await callbackRes.json().catch(() => ({}))) as Record<
        string,
        unknown
      >;
      const challenge = parsePaymentChallenge(raw, callback);
      if (allowPaymentHandler && !payment && this.paymentHandler) {
        const authorization = await this.paymentHandler(
          challenge,
          (payerWallet) =>
            this.preparePaidOperation(challenge.operationId, payerWallet)
        );
        if (challenge.bindingStatus === "provisional") {
          await this.preparePaidOperation(
            challenge.operationId,
            authorization.payer_wallet
          );
        }
        return this.submitSignedCallback(callback, authorization, false);
      }
      throw new PaymentRequiredError(challenge);
    }
    if (callbackRes.status === 202) {
      const raw = (await callbackRes.json().catch(() => ({}))) as Record<
        string,
        unknown
      >;
      const operationId =
        typeof raw.operation_id === "string"
          ? raw.operation_id
          : callback.correlation_id;
      return this.pollPaidOperation(operationId);
    }
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
    const body = (await callbackRes.json().catch(() => ({}))) as Record<
      string,
      unknown
    >;
    return normalizeSignResult(body);
  }

  private async pollPaidOperation(
    operationId: string
  ): Promise<SignMemoryResult> {
    for (let attempt = 0; attempt < 40; attempt++) {
      const status = await this.paidOperationStatus(operationId);
      if (status.status === "anchored") {
        return normalizeSignResult({
          status: "anchored",
          operation_id: status.operationId,
          attestation_id: status.attestationId,
          solana_tx: status.solanaTx,
          arweave_tx: status.arweaveTx,
          receipt: status.receipt,
          content_hash: status.binding.artifact_hash,
        });
      }
      if (
        status.status === "payment_failed" ||
        status.status === "delivery_retryable"
      ) {
        throw new ServerError(
          `paid operation ${operationId} requires retry: ${String(
            status.lastError ?? status.status
          )}`
        );
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    throw new ServerError(`paid operation ${operationId} is still pending`);
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
  // Internal: JSON-RPC over HTTP to /mcp
  // ------------------------------------------------------------------------

  /**
   * Single MCP tool call. Wraps the `tools/call` JSON-RPC envelope, attaches
   * the Bearer JWT, normalizes the wide range of server response shapes
   * (some tools return their result in `result.content[0].text` JSON-encoded,
   * others return it in `result` directly).
   */
  private async callTool(
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

function parsePaymentChallenge(
  raw: Record<string, unknown>,
  callback: SignedPaymentChallenge["callback"]
): SignedPaymentChallenge {
  if (
    (raw.status !== "payment_required" && raw.status !== "quote_refreshed") ||
    typeof raw.operation_id !== "string" ||
    typeof raw.artifact_hash !== "string" ||
    typeof raw.binding_digest !== "string" ||
    !isRecord(raw.quote)
  ) {
    throw new ServerError("payment-required response was malformed", 402);
  }
  const quote = raw.quote;
  if (
    typeof quote.amount !== "string" ||
    typeof quote.asset !== "string" ||
    typeof quote.network !== "string" ||
    typeof quote.pay_to !== "string" ||
    typeof quote.expires_at !== "string" ||
    !Array.isArray(quote.accepts) ||
    !quote.accepts.every(
      (item) =>
        isRecord(item) && (item.scheme === "stake" || item.scheme === "exact")
    )
  ) {
    throw new ServerError("payment quote was malformed", 402);
  }
  if (raw.operation_id !== callback.correlation_id) {
    throw new IntegrityError("payment quote operation_id does not match callback");
  }
  const binding = parsePaymentBinding(raw.binding);
  if (
    binding.operation_id !== raw.operation_id ||
    binding.artifact_hash !== raw.artifact_hash
  ) {
    throw new IntegrityError("payment quote binding does not match signed operation");
  }
  return {
    operationId: raw.operation_id,
    artifactHash: raw.artifact_hash,
    binding,
    bindingDigest: raw.binding_digest,
    bindingStatus:
      raw.binding_status === "final" || binding.payer_wallet !== ""
        ? "final"
        : "provisional",
    ...(typeof raw.workspace === "string" ? { workspace: raw.workspace } : {}),
    refreshed: raw.status === "quote_refreshed",
    quote: quote as unknown as PaymentQuote,
    callback: { ...callback },
  };
}

function normalizeSignResult(body: Record<string, unknown>): SignMemoryResult {
  const attestationId =
    typeof body.attestation_id === "string" ? body.attestation_id : null;
  if (!attestationId) {
    throw new IntegrityError("sign-callback did not return attestation_id");
  }
  return {
    attestationId,
    signedAt:
      typeof body.signed_at === "string"
        ? body.signed_at
        : new Date().toISOString(),
    status:
      body.status === "anchored"
        ? "anchored"
        : body.status === "pending"
        ? "pending"
        : "signed",
    ...(typeof body.content_hash === "string"
      ? { contentHash: body.content_hash }
      : {}),
    ...(typeof body.arweave_tx === "string"
      ? { arweaveTx: body.arweave_tx }
      : {}),
    ...(typeof body.solana_tx === "string"
      ? { solanaTx: body.solana_tx }
      : {}),
    ...(typeof body.operation_id === "string"
      ? { operationId: body.operation_id }
      : {}),
    ...(body.payment_receipt !== undefined
      ? { paymentReceipt: parsePaymentReceipt(body.payment_receipt) }
      : body.receipt !== undefined
      ? { paymentReceipt: parsePaymentReceipt(body.receipt) }
      : {}),
  };
}

function parsePaymentBinding(value: unknown): PaymentOperationBinding {
  if (!isRecord(value) || !isRecord(value.scope)) {
    throw new ServerError("payment operation binding was malformed");
  }
  const scope = value.scope;
  const strings = [
    "operation_id",
    "payer_subject",
    "payer_wallet",
    "artifact_hash",
    "amount",
    "asset",
    "network",
    "pay_to",
    "expires_at",
    "nonce",
  ] as const;
  if (
    value.version !== 1 ||
    strings.some((field) => typeof value[field] !== "string") ||
    (scope.visibility !== "private" && scope.visibility !== "public") ||
    !["manual", "pre_compaction", "session_end"].includes(
      String(scope.action)
    ) ||
    (scope.workspace_hash !== undefined &&
      typeof scope.workspace_hash !== "string")
  ) {
    throw new ServerError("payment operation binding was malformed");
  }
  return value as unknown as PaymentOperationBinding;
}

function parsePaymentReceipt(value: unknown): PaymentReceipt {
  if (!isRecord(value)) throw new ServerError("payment receipt was malformed");
  const strings = [
    "operation_id",
    "scheme",
    "status",
    "binding_digest",
    "payer_wallet",
    "amount",
    "asset",
    "network",
    "pay_to",
    "settled_at",
  ] as const;
  if (
    strings.some((field) => typeof value[field] !== "string") ||
    (value.scheme !== "stake" && value.scheme !== "exact") ||
    value.status !== "settled" ||
    value.receipt === undefined
  ) {
    throw new ServerError("payment receipt was malformed");
  }
  return value as unknown as PaymentReceipt;
}

function parsePaidOperationStatus(
  body: Record<string, unknown>,
  expectedOperationId: string
): PaidOperationStatus {
  const allowed = [
    "awaiting_payment",
    "payment_authorizing",
    "payment_ready",
    "anchoring",
    "verifying_delivery",
    "anchored",
    "payment_failed",
    "delivery_retryable",
  ];
  if (
    body.operation_id !== expectedOperationId ||
    typeof body.status !== "string" ||
    !allowed.includes(body.status) ||
    typeof body.binding_digest !== "string"
  ) {
    throw new ServerError("paid operation status was malformed");
  }
  const binding = parsePaymentBinding(body.binding);
  const result: PaidOperationStatus = {
    operationId: expectedOperationId,
    status: body.status as PaidOperationStatus["status"],
    binding,
    bindingDigest: body.binding_digest,
    bindingStatus:
      body.binding_status === "final" || binding.payer_wallet !== ""
        ? "final"
        : "provisional",
  };
  if (typeof body.workspace === "string") result.workspace = body.workspace;
  if (body.receipt !== null && body.receipt !== undefined) {
    result.receipt = parsePaymentReceipt(body.receipt);
  }
  if (typeof body.attestation_id === "string") result.attestationId = body.attestation_id;
  if (typeof body.solana_tx === "string") result.solanaTx = body.solana_tx;
  if (typeof body.arweave_tx === "string") result.arweaveTx = body.arweave_tx;
  if (typeof body.last_error === "string") result.lastError = body.last_error;
  return result;
}

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
