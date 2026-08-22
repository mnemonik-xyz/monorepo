// Cloud-tier thin client. Wraps the hosted MCP HTTP surface so the
// extension can ship locally-signed COSE bundles to the server's
// deferred-signing endpoint, and pull cloud-side recall/verify
// results. D2 of the tech-spec: cloud = thin client to hosted-`full`;
// no new storage formats, no server-side key material — the COSE
// bytes are signed locally and the server only persists + anchors.
//
// **Surface (T18):**
//   - `signRemote(args)`  → POST /mcp `mnemonic_sign_memory` then
//                            POST /api/sign-callback. Returns the
//                            anchor identifiers the queue drain
//                            writes back onto the local row.
//   - `recallRemote(q,k)` → POST /mcp `mnemonic_recall`. Used by the
//                            popup's Cloud-tier Recall merge.
//   - `verifyRemote(id)`  → POST /mcp `mnemonic_verify`. Used by the
//                            Verify tab in Cloud mode.
//
// **Error contract (typed so the alarm-drain can branch on type):**
//   - `ReauthRequiredError`  — 401 from any leg. The drain MUST stop
//                              and emit `re-auth-required` to the
//                              popup; the user is sent back to the
//                              T16 sign-in flow.
//   - `PermanentSyncError`   — 4xx (not 401). The row should be marked
//                              `sync_failed_permanent` and surfaced —
//                              retrying will never succeed.
//   - `TransientSyncError`   — 5xx or network failure. The drain
//                              leaves the row in `pending_uploads`;
//                              the next alarm tick retries.
//
// **Auth (D9 reminder):** the JWT goes to `/mcp` for tool dispatch.
// `/api/sign-callback` does NOT take a Bearer — capability auth via
// `correlation_id` + signature chain (matches `packages/sdk`).

import type { SignerLike, SignRemoteArgs, SignRemoteResult } from "./types.js";

/** 401 sentinel — drain MUST stop and the popup MUST re-prompt sign-in. */
export class ReauthRequiredError extends Error {
  public override readonly name = "ReauthRequiredError";
  public constructor(message = "cloud-sync requires re-authentication") {
    super(message);
  }
}

/** 4xx (non-401) — the row is rejected for good. Caller marks it
 *  `sync_failed_permanent` and stops retrying. */
export class PermanentSyncError extends Error {
  public override readonly name = "PermanentSyncError";
  public readonly status: number;
  public constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

/** 5xx or transport failure — the alarm-drain leaves the row in the
 *  queue and retries on the next tick (5-min cadence per T10). */
export class TransientSyncError extends Error {
  public override readonly name = "TransientSyncError";
  public readonly status: number | null;
  public constructor(message: string, status: number | null = null) {
    super(message);
    this.status = status;
  }
}

/** Hosted MCP origin. Phase 1 hard-code matches `auth/google-oauth.ts`;
 *  a future options-page seam will read this from `chrome.storage.sync`. */
export const DEFAULT_CLOUD_ORIGIN = "https://mcp.mnemonik.xyz";

/** Construction-time deps so tests can inject `fetch` + `now`. */
export interface CloudClientOptions {
  /** Bearer JWT for `/mcp` calls. `/api/sign-callback` deliberately
   *  goes unauthenticated — capability auth via correlation_id. */
  jwt: string;
  /** Base origin. Trailing slashes are stripped. */
  baseUrl?: string;
  /** Override `globalThis.fetch` — required for unit tests. */
  fetch?: typeof fetch;
}

export interface RecallRemoteHit {
  attestation_id: string;
  content: string;
  similarity: number;
  tags?: string[];
  signed_at?: string;
  solana_tx?: string;
  arweave_tx?: string;
}

export type VerifyRemoteResult =
  | {
      status: "verified";
      signer: string;
      arweave_tx?: string;
      solana_tx?: string;
    }
  | { status: "tampered"; signer: string; reason: string }
  | { status: "not_found" };

/** Thin wrapper over the hosted MCP surface. Construction is cheap —
 *  no network I/O. Each call is self-contained so the SW can build
 *  per-tick instances without leaking transactions across awaits. */
export class CloudClient {
  private readonly jwt: string;
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;

  public constructor(opts: CloudClientOptions) {
    if (!opts.jwt) {
      throw new Error("CloudClient: jwt is required");
    }
    this.jwt = opts.jwt;
    this.baseUrl = (opts.baseUrl ?? DEFAULT_CLOUD_ORIGIN).replace(/\/+$/, "");
    this.fetchImpl = opts.fetch ?? globalThis.fetch.bind(globalThis);
  }

  /**
   * Deferred-signing flow per D2.
   *
   *   1. `POST /mcp` `mnemonic_sign_memory` (Bearer JWT) →
   *      `{correlation_id, expires_in}`.
   *   2. `POST /api/sign-callback` (NO Bearer — capability auth) with
   *      `{correlation_id, cose_signed_bytes (b64), signer_pubkey}` →
   *      `{attestation_id, solana_tx, arweave_tx, ...}`.
   *
   * Returns the three identifiers the alarm-drain writes back onto
   * the local `AttestationRow`. Throws one of the three typed errors
   * above so the caller can branch without string parsing.
   */
  public async signRemote(
    args: SignRemoteArgs,
    signer: SignerLike,
  ): Promise<SignRemoteResult> {
    const openBody = {
      content: args.content,
      // Automatic/background extension sync has no wallet-approved paid
      // session context. Pin it to free local mode so it can never trigger an
      // anchoring payment. A future explicit Anchor action will pass a scoped
      // session authorization through the SDK instead.
      mode: "local" as const,
      ...(args.tags.length > 0 ? { tags: args.tags } : {}),
      ...(args.source_meta ? { source_meta: args.source_meta } : {}),
    };
    const open = await this.rpc<{
      correlation_id?: unknown;
      expires_in?: unknown;
    }>("mnemonic_sign_memory", openBody);
    // Reject empty strings explicitly — a malformed server response
    // that returns `correlation_id: ""` would otherwise sail past the
    // `typeof === 'string'` guard and burn a sign-callback round-trip
    // only to come back as a confusing 410. Code-review round-1 T18-C-02.
    const correlationId =
      typeof open.correlation_id === "string" && open.correlation_id.length > 0
        ? open.correlation_id
        : null;
    if (!correlationId) {
      throw new PermanentSyncError(
        "mnemonic_sign_memory did not return correlation_id",
        500,
      );
    }

    // `signer.cose_bytes` was produced locally during the original
    // `signMemory` call and is already canonical-CBOR + COSE_Sign1.
    // We re-use it verbatim — the server validates it against the
    // pending bundle's stored content hash.
    const callbackUrl = `${this.baseUrl}/api/sign-callback`;
    const res = await this.fetchOr(callbackUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        correlation_id: correlationId,
        cose_signed_bytes: bytesToBase64(signer.cose_bytes),
        signer_pubkey: signer.signer_pubkey,
      }),
    });
    if (res.status === 401) {
      throw new ReauthRequiredError("sign-callback rejected with 401");
    }
    if (res.status === 410 || res.status === 409) {
      // 410: pending entry expired (server LRU dropped it). Treat
      // as permanent — the row's correlation_id is now invalid.
      throw new PermanentSyncError(
        `sign-callback consumed/expired (HTTP ${res.status})`,
        res.status,
      );
    }
    if (res.status >= 500) {
      throw new TransientSyncError(
        `sign-callback failed: HTTP ${res.status}`,
        res.status,
      );
    }
    if (!res.ok) {
      throw new PermanentSyncError(
        `sign-callback rejected: HTTP ${res.status}`,
        res.status,
      );
    }
    const body = (await safeJson(res)) as Record<string, unknown>;
    const attestationId =
      typeof body.attestation_id === "string" ? body.attestation_id : null;
    if (!attestationId) {
      throw new PermanentSyncError("sign-callback omitted attestation_id", 500);
    }
    return {
      attestation_id: attestationId,
      solana_tx: typeof body.solana_tx === "string" ? body.solana_tx : "",
      arweave_tx: typeof body.arweave_tx === "string" ? body.arweave_tx : "",
    };
  }

  /** Cloud-side recall. Returns the wire shape `RecallRemoteHit[]`. */
  public async recallRemote(
    query: string,
    k: number,
  ): Promise<RecallRemoteHit[]> {
    const trimmed = query.trim();
    if (trimmed === "" || k <= 0) return [];
    const result = await this.rpc<{
      hits?: unknown;
      results?: unknown;
    }>("mnemonic_recall", { query: trimmed, top_k: k });
    const arr = Array.isArray(result.hits)
      ? result.hits
      : Array.isArray(result.results)
        ? result.results
        : [];
    const out: RecallRemoteHit[] = [];
    for (const h of arr) {
      if (typeof h !== "object" || h === null) continue;
      const r = h as Record<string, unknown>;
      const id = typeof r.attestation_id === "string" ? r.attestation_id : "";
      if (!id) continue;
      const hit: RecallRemoteHit = {
        attestation_id: id,
        content: typeof r.content === "string" ? r.content : "",
        similarity: typeof r.similarity === "number" ? r.similarity : 0,
      };
      if (Array.isArray(r.tags)) {
        hit.tags = r.tags.filter((t): t is string => typeof t === "string");
      }
      if (typeof r.signed_at === "string") hit.signed_at = r.signed_at;
      if (typeof r.solana_tx === "string") hit.solana_tx = r.solana_tx;
      if (typeof r.arweave_tx === "string") hit.arweave_tx = r.arweave_tx;
      out.push(hit);
    }
    return out;
  }

  /** Cloud-side verify. Returns a discriminated union mirroring the
   *  SDK's `VerifyResult` (snake_case for in-extension consumers). */
  public async verifyRemote(
    attestationId: string,
  ): Promise<VerifyRemoteResult> {
    if (!attestationId) return { status: "not_found" };
    const result = await this.rpc<Record<string, unknown>>("mnemonic_verify", {
      attestation_id: attestationId,
    });
    const status =
      typeof result.status === "string" ? result.status : "not_found";
    if (status === "verified") {
      const out: VerifyRemoteResult = {
        status: "verified",
        signer: typeof result.signer === "string" ? result.signer : "",
      };
      if (typeof result.arweave_tx === "string") {
        out.arweave_tx = result.arweave_tx;
      }
      if (typeof result.solana_tx === "string") {
        out.solana_tx = result.solana_tx;
      }
      return out;
    }
    if (status === "tampered") {
      return {
        status: "tampered",
        signer: typeof result.signer === "string" ? result.signer : "",
        reason: typeof result.reason === "string" ? result.reason : "unknown",
      };
    }
    return { status: "not_found" };
  }

  // ── internals ────────────────────────────────────────────────────────

  /**
   * Single JSON-RPC `tools/call` to `/mcp`. Returns the unwrapped
   * tool result object. Throws typed errors so the alarm-drain can
   * branch on `instanceof`.
   *
   * Note: MCP `tools/call` results sometimes arrive wrapped in
   * `{content: [{type:'text', text:'<JSON>'}]}` (canonical MCP wire
   * format). Mirror the SDK's `extractToolResult` shim so the call
   * site reads a plain record either way.
   */
  private async rpc<T extends Record<string, unknown>>(
    name: string,
    args: Record<string, unknown>,
  ): Promise<T> {
    const url = `${this.baseUrl}/mcp`;
    const body = {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name, arguments: args },
    };
    const res = await this.fetchOr(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json",
        Authorization: `Bearer ${this.jwt}`,
      },
      body: JSON.stringify(body),
    });
    if (res.status === 401) {
      throw new ReauthRequiredError(`${name} rejected with 401`);
    }
    if (res.status >= 500) {
      throw new TransientSyncError(
        `${name} failed: HTTP ${res.status}`,
        res.status,
      );
    }
    if (!res.ok) {
      throw new PermanentSyncError(
        `${name} rejected: HTTP ${res.status}`,
        res.status,
      );
    }
    const parsed = (await safeJson(res)) as Record<string, unknown>;
    if (parsed.error && typeof parsed.error === "object") {
      const errObj = parsed.error as Record<string, unknown>;
      const code = typeof errObj.code === "number" ? errObj.code : null;
      const msg = typeof errObj.message === "string" ? errObj.message : name;
      if (code === 401) throw new ReauthRequiredError(msg);
      if (code !== null && code >= 500) {
        throw new TransientSyncError(msg, code);
      }
      if (code !== null) {
        throw new PermanentSyncError(msg, code);
      }
      // No JSON-RPC error code at all — the wire is malformed. Treat
      // as TransientSyncError so the alarm retries rather than
      // stamping `sync_failed_permanent` on a row that may succeed
      // on the next tick. Closes code-review round-1 nit T18-C-07.
      throw new TransientSyncError(`${msg} (no rpc error code)`, res.status);
    }
    return extractToolResult<T>(parsed.result);
  }

  /** `fetch` wrapper that rewrites network errors as
   *  `TransientSyncError` (so the SW handler doesn't have to
   *  duplicate the try/catch). */
  private async fetchOr(url: string, init: RequestInit): Promise<Response> {
    try {
      return await this.fetchImpl(url, init);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      throw new TransientSyncError(`network error: ${msg}`);
    }
  }
}

// ── helpers ────────────────────────────────────────────────────────────

function extractToolResult<T extends Record<string, unknown>>(
  result: unknown,
): T {
  if (result === null || typeof result !== "object") return {} as T;
  const r = result as Record<string, unknown>;
  // MCP `tools/call` canonical envelope.
  if (Array.isArray(r.content) && r.content.length > 0) {
    const first = r.content[0];
    if (
      first !== null &&
      typeof first === "object" &&
      typeof (first as { text?: unknown }).text === "string"
    ) {
      try {
        return JSON.parse((first as { text: string }).text) as T;
      } catch {
        return {} as T;
      }
    }
  }
  return r as T;
}

async function safeJson(res: Response): Promise<unknown> {
  try {
    return await res.json();
  } catch {
    return {};
  }
}

/** Encode bytes as standard-alphabet, padded base64. Chunked to avoid
 *  call-stack limits on very large arrays (>~64KB). */
function bytesToBase64(bytes: Uint8Array): string {
  let s = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    s += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(s);
}

// `flushPending` (the SW-facing entrypoint) now lives in
// `src/background/cloud-sync.ts` next to the drain logic. The SW
// imports from there directly.
