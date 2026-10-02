import {decodeAttestation,envelopeHex,fetchAttestation,verifyParent} from "./a2a-recovery.js";
import type {RecoveryHost} from "./a2a-recovery.js";
// A2A attestation helpers — mixin methods attached to MnemonicClient.
//
// All four methods wrap `callTool("mnemonic_attest_a2a" | "mnemonic_recall_a2a", ...)`
// through the same MCP HTTP surface used by `signMemory` / `recall`.
//
// attestA2ATask / attestA2AMessage / attestA2AArtifact
//   → POST /mcp  tools/call mnemonic_attest_a2a
//   → returns `AttestationId`
//
// recallA2AContext
//   → POST /mcp  tools/call mnemonic_recall_a2a
//   → returns `Attestation[]`
//
// The methods are declared here and merged onto MnemonicClient in client.ts
// via `Object.assign(MnemonicClient.prototype, a2aMethods)`. They share the
// private `callTool` surface via the `WithCallTool` interface below.
//
// ## Sealed A2A DataPart (T14 §9.1)
//
// `SEALED_CBOR_MEDIA_TYPE` is the media type for sealed A2A DataParts.
// `buildSealedDataPart` and `extractSealedDataPart` are pure helpers that
// construct / parse the `{ sealed, grants[] }` payload without touching the
// server — decryption remains client-side.

import { IntegrityError, ServerError, UserError } from "./errors.js";
import { loadWasm } from "./wasm.js";
import type { KeypairJson } from "./keypair.js";
import type {
  A2AArtifact,
  A2AMessage,
  A2ATask,
  AttestA2AOptions,
  AttestationId,
  Attestation,
  RecallA2AContextOptions,
} from "./types.js";

// ── Internal interface so a2aMethods can reach into MnemonicClient ───────────

/**
 * Minimal interface that the A2A mixin requires from its host class.
 * `MnemonicClient` satisfies this contract.
 */
export interface WithCallTool extends RecoveryHost {
  _resolveA2AKeypair(): Promise<KeypairJson>;
  /** Exposed as a protected-by-convention underscore method for mixin access. */
  _callToolA2A(name: string, args: Record<string, unknown>): Promise<unknown>;
}

// ── Helper ──────────────────────────────────────────────────────────────────

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Parse the `attestation_id` out of a `mnemonic_attest_a2a` response.
 */
function parseAttestationId(result: unknown, tool: string): AttestationId {
  const raw = isRecord(result) ? result : {};
  const id = typeof raw.attestation_id === "string" ? raw.attestation_id : null;
  if (!id) {
    throw new ServerError(
      `${tool}: server did not return attestation_id; got ${JSON.stringify(raw)}`
    );
  }
  return id;
}

/**
 * Parse `Attestation[]` out of a `mnemonic_recall_a2a` response.
 */
function parseAttestations(result: unknown): Attestation[] {
  const raw = isRecord(result) ? result : {};
  const items = Array.isArray(raw.attestations)
    ? raw.attestations
    : Array.isArray(raw.hits)
    ? raw.hits
    : [];
  return items.filter(isRecord).map((item) => {
    if (typeof item.attestation_id !== "string" ||
        !["task","message","artifact"].includes(String(item.kind)) ||
        typeof item.signed_at !== "string" || !isRecord(item.payload)) {
      throw new ServerError("recallA2AContext: malformed attestation metadata");
    }
    return {
      attestationId: item.attestation_id,
      kind: item.kind as "task" | "message" | "artifact",
      signedAt: item.signed_at,
      payload: item.payload,
      ...(typeof item.context_id === "string" ? {contextId:item.context_id}:{}),
      ...(typeof item.prev_id === "string" || item.prev_id === null ? {prevId:item.prev_id}:{}),
      ...(typeof item.cose_envelope_hex === "string" ? {coseEnvelopeHex:item.cose_envelope_hex}:{}),
      ...(typeof item.content_hash === "string" ? {contentHash:item.content_hash}:{}),
      ...(typeof item.signer_pubkey === "string" ? {signerPubkey:item.signer_pubkey}:{}),
      ...(typeof item.sealed === "boolean" ? {sealed:item.sealed}:{}),
      ...(isRecord(item.stream) ? {stream:item.stream as unknown as import("./types.js").SealedA2AStream}:{}),
      ...(isRecord(item.sealed_payload) ? {sealedPayload:item.sealed_payload as unknown as {sealed:string;grants:string[]}}:{}),
    };
  });
}

// ── A2A method implementations ───────────────────────────────────────────────

/**
 * Attest an A2A task object. Serialises the task to JSON, sends it to
 * `mnemonic_attest_a2a` on the MCP server, and returns the opaque
 * `attestation_id`.
 */
export async function attestA2ATask(
  this: WithCallTool,
  task: A2ATask,
  contextId: string,
  opts: AttestA2AOptions = {}
): Promise<AttestationId> {
  if (!task || !task.id) {
    throw new UserError("attestA2ATask: task must have a non-empty id");
  }
  if (!contextId || typeof contextId !== "string") {
    throw new UserError("attestA2ATask: contextId must be a non-empty string");
  }
  const result = await sendSignedA2A.call(this, "task", task, contextId, opts);
  return parseAttestationId(result, "attestA2ATask");
}

/**
 * Attest an A2A message object and return the `attestation_id`.
 */
export async function attestA2AMessage(
  this: WithCallTool,
  msg: A2AMessage,
  contextId: string,
  opts: AttestA2AOptions = {}
): Promise<AttestationId> {
  if (!msg || !msg.messageId) {
    throw new UserError("attestA2AMessage: msg must have a non-empty messageId");
  }
  if (!contextId || typeof contextId !== "string") {
    throw new UserError("attestA2AMessage: contextId must be a non-empty string");
  }
  const result = await sendSignedA2A.call(this, "message", msg, contextId, opts);
  return parseAttestationId(result, "attestA2AMessage");
}

/**
 * Attest an A2A artifact object and return the `attestation_id`.
 */
export async function attestA2AArtifact(
  this: WithCallTool,
  art: A2AArtifact,
  contextId: string,
  opts: AttestA2AOptions = {}
): Promise<AttestationId> {
  if (!art || !art.artifactId) {
    throw new UserError(
      "attestA2AArtifact: artifact must have a non-empty artifactId"
    );
  }
  if (!contextId || typeof contextId !== "string") {
    throw new UserError(
      "attestA2AArtifact: contextId must be a non-empty string"
    );
  }
  const result = await sendSignedA2A.call(this, "artifact", art, contextId, opts);
  return parseAttestationId(result, "attestA2AArtifact");
}

/**
 * Recall attestations for a given A2A context.
 *
 * Returns all attestations (tasks, messages, artifacts) anchored under
 * `contextId`, optionally filtered by `kind` and limited to `limit` items.
 */
export async function recallA2AContext(
  this: WithCallTool,
  contextId: string,
  opts: RecallA2AContextOptions = {}
): Promise<Attestation[]> {
  if (!contextId || typeof contextId !== "string") {
    throw new UserError(
      "recallA2AContext: contextId must be a non-empty string"
    );
  }
  if(opts.mode!==undefined&&!['local','anchored'].includes(opts.mode))throw new UserError('invalid A2A mode');
  const filter=(rows:Attestation[])=>rows.filter(r=>r.contextId===contextId&&(!opts.kind||opts.kind==='all'||r.kind===opts.kind)&&(opts.sealed===undefined||r.sealed===opts.sealed)).sort((a,b)=>Date.parse(b.signedAt)-Date.parse(a.signedAt)||b.attestationId.localeCompare(a.attestationId)).slice(0,opts.limit??100);
  if(opts.limit!==undefined&&(!Number.isInteger(opts.limit)||opts.limit<1||opts.limit>1000))throw new UserError('limit must be 1..1000');
  if(opts.mode==='local')return filter(await this._a2aIndexStore().list());
  const args: Record<string, unknown> = { context_id: contextId };
  if (typeof opts.sealed === "boolean") args.sealed = opts.sealed;
  if (typeof opts.limit === "number") args.limit = opts.limit;
  if (opts.kind && opts.kind !== "all") args.kind = opts.kind;
  const result = await this._callToolA2A("mnemonic_recall_a2a", args);
  if(isRecord(result)&&Array.isArray(result.receipts)){
    const rows:Attestation[]=[];
    for(const receipt of result.receipts){
      if(!isRecord(receipt)||typeof receipt.locator!=='string'||typeof receipt.signer_pubkey!=='string')throw new IntegrityError('invalid receipt');
      const row=await fetchAttestation.call(this,receipt.locator,receipt.signer_pubkey);
      if(row.attestationId!==receipt.attestation_id||row.contentHash!==receipt.content_hash||row.contextId!==contextId||row.kind!==receipt.kind||row.signedAt!==receipt.signed_at||row.prevId!==receipt.prev_id||row.sealed!==receipt.sealed)throw new IntegrityError('receipt binding mismatch');
      await this._a2aIndexStore().put(row);rows.push(row);
    }return filter(rows);
  }
  return parseAttestations(result);
}

// ── Sealed A2A DataPart helpers (T14 §9.1) ───────────────────────────────────

/**
 * Media type for a sealed-memory A2A DataPart.
 *
 * DataParts with this `mimeType` carry a `{ sealed, grants[] }` payload where
 * `sealed` is the base64-encoded COSE-signed SEALED_V1 CBOR bytes and `grants` is an
 * array of base64-encoded COSE-signed GRANT_V1 CBOR blobs.
 *
 * Decryption is always client-side — the operator never sees the content key.
 */
export const SEALED_CBOR_MEDIA_TYPE =
  "application/vnd.mnemonic.sealed+cbor" as const;

/**
 * Wire payload of a sealed A2A DataPart.
 */
export interface SealedA2APartPayload {
  /** Base64-encoded COSE-signed SEALED_V1 CBOR bytes. */
  sealed: string;
  /** Base64-encoded COSE-signed GRANT_V1 CBOR blobs — one per authorised reader. */
  grants: string[];
}

/**
 * A2A DataPart shape (kind="data") used to carry a sealed memory.
 */
export interface SealedA2ADataPart {
  kind: "data";
  data: SealedA2APartPayload;
  mimeType: typeof SEALED_CBOR_MEDIA_TYPE;
}

/**
 * Build an A2A DataPart that carries sealed-memory content.
 *
 * @param sealedCbor - COSE-signed SEALED_V1 bytes.
 * @param grants - Optional list of COSE-signed GRANT_V1 byte arrays to attach.
 * @returns An A2A DataPart with `mimeType = SEALED_CBOR_MEDIA_TYPE`.
 */
export function buildSealedDataPart(
  sealedCbor: Uint8Array,
  grants: Uint8Array[] = []
): SealedA2ADataPart {
  const sealedB64 = uint8ArrayToBase64(sealedCbor);
  const grantsB64 = grants.map((g) => uint8ArrayToBase64(g));
  return {
    kind: "data",
    data: { sealed: sealedB64, grants: grantsB64 },
    mimeType: SEALED_CBOR_MEDIA_TYPE,
  };
}

/**
 * Extract `{ sealedCbor, grants }` from an A2A DataPart carrying
 * `SEALED_CBOR_MEDIA_TYPE`.
 *
 * @throws `UserError` if the part is not a sealed DataPart.
 */
export function extractSealedDataPart(part: {
  kind: string;
  data?: unknown;
  mimeType?: string;
}): { sealedCbor: Uint8Array; grants: Uint8Array[] } {
  if (part.kind !== "data") {
    throw new UserError(
      `extractSealedDataPart: expected kind="data", got "${part.kind}"`
    );
  }
  if (part.mimeType !== SEALED_CBOR_MEDIA_TYPE) {
    throw new UserError(
      `extractSealedDataPart: expected mimeType="${SEALED_CBOR_MEDIA_TYPE}", ` +
        `got "${part.mimeType ?? "(none)"}"`
    );
  }
  if (!isRecord(part.data)) {
    throw new UserError("extractSealedDataPart: data field is not an object");
  }
  const payload = part.data as Record<string, unknown>;
  if (typeof payload.sealed !== "string") {
    throw new UserError(
      'extractSealedDataPart: data.sealed must be a base64 string'
    );
  }
  const grantsRaw = Array.isArray(payload.grants) ? payload.grants : [];
  const sealedCbor = base64ToUint8Array(payload.sealed);
  const grants = grantsRaw.map((g, i) => {
    if (typeof g !== "string") {
      throw new UserError(
        `extractSealedDataPart: grants[${i}] must be a base64 string`
      );
    }
    return base64ToUint8Array(g);
  });
  return { sealedCbor, grants };
}

// ── Base64 utilities ──────────────────────────────────────────────────────────

/** Encode a Uint8Array to standard (padded) base64. */
function uint8ArrayToBase64(bytes: Uint8Array): string {
  // Use btoa via a temporary binary string.
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]!);
  }
  return btoa(binary);
}

/** Decode a standard (padded) base64 string to Uint8Array. */
function base64ToUint8Array(b64: string): Uint8Array {
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}


function fromHex(hex: string): Uint8Array {
  if (!/^(?:[0-9a-fA-F]{2})+$/.test(hex)) throw new UserError("invalid A2A envelope hex");
  return Uint8Array.from(hex.match(/../g)!.map((s) => parseInt(s,16)));
}

async function sendSignedA2A(this: WithCallTool, kind: string, payload: unknown, context: string, opts: AttestA2AOptions): Promise<unknown> {
  if(opts.mode!==undefined&&!['local','anchored'].includes(opts.mode))throw new UserError('invalid A2A mode');
  if(opts.prevLocator&&!opts.prevId)throw new UserError('root cannot have parent locator');
  const wasm = await loadWasm();
  if (!wasm.prepare_a2a) throw new ServerError("A2A WASM bindings are unavailable; rebuild the SDK");
  const recipients = opts.sealed?.recipients.map((r) => ({card:r.card,trusted_card_signer:r.trustedCardSigner}));
  const kp = await this._resolveA2AKeypair();
  const author=kp.pubkey_base58;
  let bytes: Uint8Array;
  try {
    bytes = wasm.prepare_a2a(kp,kind,JSON.stringify(payload),context,opts.prevId,new Date().toISOString(),
      recipients ? JSON.stringify(recipients):undefined,opts.sealed?.chunkSize);
  } finally { kp.secret.fill(0); }
  const signed = Array.from(bytes,(b)=>b.toString(16).padStart(2,"0")).join("");
  const row=await decodeAttestation(bytes,author);
  const parent=opts.prevId?(await this._a2aIndexStore().list()).find(r=>r.attestationId===opts.prevId):undefined;
  if(parent)await verifyParent(row,parent);
  if(opts.mode==='local'){
    if(opts.prevId&&!parent)throw new UserError('local parent missing');
    await this._a2aIndexStore().put(row);return {attestation_id:row.attestationId};
  }
  const locator=opts.prevLocator??parent?.locator;
  if(opts.prevId&&!locator)throw new UserError('ParentLocatorRequired');
  const result=await this._callToolA2A("mnemonic_attest_a2a",{kind,context_id:context,signed,sealed:!!opts.sealed,mode:'anchored',...(opts.prevId?{prev_id:opts.prevId}:{}),...(locator?{prev_locator:locator}:{})});
  if(isRecord(result)&&typeof result.locator==='string'){
    if(result.attestation_id!==row.attestationId)throw new IntegrityError('upload receipt hash mismatch');
    const delivered=await fetchAttestation.call(this,result.locator,author);
    if(delivered.coseEnvelopeHex!==signed)throw new IntegrityError('upload delivery mismatch');
    await this._a2aIndexStore().put(delivered);
  }return result;
}

/** Verify author and the complete sealed chain, then decrypt locally. */
export async function openA2AAttestation(this: WithCallTool, attestation: Attestation, expectedAuthor: string, encryptionSecret?: Uint8Array): Promise<Record<string,unknown>> {
  if (!attestation.coseEnvelopeHex) throw new UserError("A2A recall omitted signed bytes");
  const wasm = await loadWasm();
  if (!wasm.open_a2a || !wasm.verify_a2a) throw new ServerError("A2A WASM bindings unavailable");
  const signed = fromHex(attestation.coseEnvelopeHex);
  try {
    const verified = await verifyA2AAttestation(attestation.coseEnvelopeHex,expectedAuthor);
    if (attestation.contentHash !== verified.content_hash || attestation.signerPubkey !== expectedAuthor ||
        attestation.kind !== verified.binding.kind || attestation.signedAt !== verified.binding.created_at || attestation.sealed !== verified.binding.sealed ||
        (attestation.contextId !== undefined && attestation.contextId !== verified.binding.context_id) ||
        (attestation.prevId !== undefined && attestation.prevId !== verified.binding.prev_id) ||
        attestation.attestationId !== `a2a:${verified.content_hash}`) {
      throw new Error("recall metadata does not match signed binding");
    }
    const kp = await this._resolveA2AKeypair();
    try {
      return JSON.parse(wasm.open_a2a(kp,signed,expectedAuthor,encryptionSecret)) as Record<string,unknown>;
    } finally { kp.secret.fill(0); }
  } catch (e) { throw new IntegrityError("A2A verification or decryption failed",e); }
}

/** Public verification requires only signed bytes and a trusted author key. */
export async function verifyA2AAttestation(coseEnvelopeHex: string, expectedAuthor: string): Promise<{
  content_hash: string; signer: string; binding: {kind: string; created_at: string; sealed: boolean; context_id: string; prev_id: string | null; stream?: import("./types.js").SealedA2AStream; payload: Record<string,unknown>};
}> {
  if (!expectedAuthor) throw new UserError("a trusted A2A author is required");
  const wasm = await loadWasm();
  if (!wasm.verify_a2a) throw new ServerError("A2A WASM bindings unavailable");
  try { return JSON.parse(wasm.verify_a2a(fromHex(coseEnvelopeHex),expectedAuthor)); }
  catch(e) { throw new IntegrityError("A2A public verification failed",e); }
}
