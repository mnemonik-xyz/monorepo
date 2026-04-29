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
  ServerError,
  UserError,
  redactJWT,
} from "./errors.js";
import type { Keypair, KeypairJson } from "./keypair.js";
import type {
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
  }

  /** Set or replace the JWT after construction (e.g. after OAuth login). */
  setJwt(jwt: string | undefined): void {
    this.jwt = jwt;
  }

  /**
   * Bind a `Keypair` so that `signMemory` can produce the COSE envelope.
   * Required only for sign flows; recall/verify/whoami/prove do not need it.
   */
  setKeypair(keypair: Keypair): void {
    this.keypairJson = keypair.toJSON();
  }

  // ------------------------------------------------------------------------
  // Tool methods
  // ------------------------------------------------------------------------

  /**
   * Server-side `mnemonic_whoami`. Returns the **server's** identity, NOT
   * the user's (Decision 14 — CLI implements its own whoami client-side).
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
   * Sign a memory. Always uses the pending-bundle / sign-callback flow.
   *
   * Throws `UserError` if no keypair is bound (call `setKeypair` first).
   * Throws `AuthError` on 401, `ServerError` on 5xx / network failure,
   * `IntegrityError` if the server's content_hash header disagrees with
   * the COSE envelope after signing (defense-in-depth: the server
   * re-verifies, but failing fast here gives a better error).
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

  /** Server-side `mnemonic_recall`. */
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

  /** Server-side `mnemonic_verify`. */
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

  /** Server-side `mnemonic_prove_identity`. (Not used by CLI — Decision 14.) */
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
    return redactJWT(txt.slice(0, 500));
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
