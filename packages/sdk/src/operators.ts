/** Explicit operator selection. This is local configuration, not a public directory. */
import { verifyAsync } from '@noble/ed25519';
import { MnemonicClient } from './client.js';
import { checkpointPublicKey } from './checkpoint.js';
import { IntegrityError, ServerError, UserError } from './errors.js';
import type { MnemonicClientConfig } from './types.js';

export interface OperatorConfig {
  id: string;
  /** HTTPS origin; no /mcp suffix, credentials, query or fragment. */
  baseUrl: string;
  /** Independently obtained base58 Ed25519 public key, not learned from the endpoint. */
  publicKey: string;
}
export interface OperatorList { version: 1; operators: OperatorConfig[] }

function object(value: unknown, keys: string[]): asserts value is Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).sort().join(',') !== keys.sort().join(',')) {
    throw new UserError('invalid operator configuration fields');
  }
}

/** Parse untrusted JSON, rejecting ambiguous IDs, origins and shared identities. */
export function parseOperatorList(value: unknown): OperatorList {
  object(value, ['version', 'operators']);
  if (value.version !== 1 || !Array.isArray(value.operators) ||
      value.operators.length === 0 || value.operators.length > 32) {
    throw new UserError('operator list requires version 1 and 1–32 operators');
  }
  const ids = new Set<string>(), origins = new Set<string>(), pins = new Set<string>();
  const operators = value.operators.map((row: unknown): OperatorConfig => {
    object(row, ['id', 'baseUrl', 'publicKey']);
    if (typeof row.id !== 'string' || !/^[a-z][a-z0-9-]{0,63}$/.test(row.id) ||
        typeof row.baseUrl !== 'string' || typeof row.publicKey !== 'string') {
      throw new UserError('invalid operator ID, origin or public key');
    }
    let url: URL;
    try { url = new URL(row.baseUrl); }
    catch { throw new UserError('invalid operator HTTPS origin'); }
    if (url.protocol !== 'https:' || row.baseUrl !== url.origin) {
      throw new UserError('operator baseUrl must be an HTTPS origin without a path');
    }
    checkpointPublicKey(row.publicKey);
    if (ids.has(row.id) || origins.has(url.origin) || pins.has(row.publicKey)) {
      throw new UserError('operators must have distinct IDs, origins and public keys');
    }
    ids.add(row.id); origins.add(url.origin); pins.add(row.publicKey);
    return { id: row.id, baseUrl: url.origin, publicKey: row.publicKey };
  });
  return { version: 1, operators };
}

/** Domain tag of the server-composed operator proof message. */
export const OPERATOR_PROOF_DOMAIN = 'mnemonic.operator-selection.v1';
const DEFAULT_PROOF_TIMEOUT_MS = 15_000;

export interface OperatorProofOptions {
  /** Upper bound for the identity proof request. Default 15000 ms, maximum 120000 ms. */
  proofTimeoutMs?: number;
  /** Optional caller abort signal for the identity proof request. */
  signal?: AbortSignal;
  /** Fetch override, mainly for tests. Receives no credentials. */
  fetch?: typeof fetch;
}

/**
 * Verify a fresh identity proof from one configured operator without sending
 * any credential. The server signs `${OPERATOR_PROOF_DOMAIN}\n${origin}\n${nonce}`;
 * the message is checked against the configured origin and independent pin.
 */
export async function verifyOperatorIdentity(
  operator: Readonly<OperatorConfig>,
  options: OperatorProofOptions = {},
): Promise<void> {
  const timeout = options.proofTimeoutMs ?? DEFAULT_PROOF_TIMEOUT_MS;
  if (!Number.isInteger(timeout) || timeout <= 0 || timeout > 120_000) {
    throw new UserError('proofTimeoutMs must be an integer from 1 to 120000');
  }
  const fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  const signal = options.signal ?
    AbortSignal.any([options.signal, AbortSignal.timeout(timeout)]) : AbortSignal.timeout(timeout);
  const nonce = Array.from(crypto.getRandomValues(new Uint8Array(32)),
    byte => byte.toString(16).padStart(2, '0')).join('');
  let proof: Record<string, unknown>;
  try {
    // Deliberately bare: no Authorization header, cookies or token refresh.
    const res = await fetchImpl(`${operator.baseUrl}/mcp`, {
      method: 'POST', redirect: 'error', credentials: 'omit', signal,
      headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call',
        params: { name: 'mnemonic_operator_proof', arguments: { nonce } } }),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const envelope = await res.json() as { result?: { content?: { text?: unknown }[] } & Record<string, unknown> };
    const text = envelope.result?.content?.[0]?.text;
    proof = typeof text === 'string' ? JSON.parse(text) : envelope.result ?? {};
  } catch (cause) {
    throw new ServerError('operator identity proof request failed', undefined, cause);
  }
  if (proof.public_key !== operator.publicKey || proof.origin !== operator.baseUrl ||
      proof.nonce !== nonce || typeof proof.signature !== 'string' ||
      !/^[0-9a-f]{128}$/i.test(proof.signature)) {
    throw new IntegrityError('operator identity proof does not match configured pin');
  }
  const message = `${OPERATOR_PROOF_DOMAIN}\n${operator.baseUrl}\n${nonce}`;
  const signature = Uint8Array.from(proof.signature.match(/../g)!.map(byte => parseInt(byte, 16)));
  if (!await verifyAsync(signature, new TextEncoder().encode(message), checkpointPublicKey(operator.publicKey))) {
    throw new IntegrityError('invalid operator identity signature');
  }
}

/**
 * Connect to exactly one configured operator after verifying a fresh identity proof.
 * The proof request carries no credentials and is bounded by `proofTimeoutMs`;
 * the authenticated client is created only after the proof succeeds.
 * Each call creates a new client. It never changes an existing client's endpoint,
 * transfers its JWT, retries its operations or falls back to another operator.
 * Pass credentials/token refreshers scoped to this operator only.
 * Proof authenticates key possession at connection time, not capability or uptime.
 */
export async function connectOperator(
  list: OperatorList,
  id: string,
  config: Omit<MnemonicClientConfig, 'baseUrl'>,
  options: Omit<OperatorProofOptions, 'fetch'> = {},
): Promise<{ operator: Readonly<OperatorConfig>; client: MnemonicClient }> {
  const operator = parseOperatorList(list).operators.find(row => row.id === id);
  if (!operator) throw new UserError('operator ID is not configured');
  const fetchImpl = config.fetch ?? globalThis.fetch.bind(globalThis);
  await verifyOperatorIdentity(operator, { ...options, fetch: fetchImpl });
  const client = new MnemonicClient({
    ...config,
    baseUrl: operator.baseUrl,
    // The selected MCP origin cannot redirect requests (including paid writes)
    // to another operator. Storage fetches retain their existing behavior.
    fetch: (input, init) => {
      const url = new URL(input instanceof Request ? input.url : String(input));
      return fetchImpl(input, url.origin === operator.baseUrl ? { ...init, redirect: 'error' } : init);
    },
  });
  return { operator: Object.freeze(operator), client };
}
