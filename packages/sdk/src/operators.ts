/** Explicit operator selection. This is local configuration, not a public directory. */
import { verifyAsync } from '@noble/ed25519';
import { MnemonicClient } from './client.js';
import { checkpointPublicKey } from './checkpoint.js';
import { IntegrityError, UserError } from './errors.js';
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

/**
 * Connect to exactly one configured operator and verify a fresh identity proof.
 * Each call creates a new client. It never changes an existing client's endpoint,
 * transfers its JWT, retries its operations or falls back to another operator.
 * Pass credentials/token refreshers scoped to this operator only.
 * Proof authenticates key possession at connection time, not capability or uptime.
 */
export async function connectOperator(
  list: OperatorList,
  id: string,
  config: Omit<MnemonicClientConfig, 'baseUrl'>,
): Promise<{ operator: Readonly<OperatorConfig>; client: MnemonicClient }> {
  const operator = parseOperatorList(list).operators.find(row => row.id === id);
  if (!operator) throw new UserError('operator ID is not configured');
  const fetchImpl = config.fetch ?? globalThis.fetch.bind(globalThis);
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
  const nonce = Array.from(crypto.getRandomValues(new Uint8Array(32)),
    byte => byte.toString(16).padStart(2, '0')).join('');
  const challenge = `mnemonic.operator-selection.v1\n${operator.baseUrl}\n${nonce}`;
  const proof = await client.proveIdentity(challenge);
  if (proof.pubkey !== operator.publicKey || proof.challenge !== challenge ||
      !/^[0-9a-f]{128}$/i.test(proof.signature)) {
    throw new IntegrityError('operator identity proof does not match configured pin');
  }
  const signature = Uint8Array.from(proof.signature.match(/../g)!.map(byte => parseInt(byte, 16)));
  if (!await verifyAsync(signature, new TextEncoder().encode(challenge), checkpointPublicKey(operator.publicKey))) {
    throw new IntegrityError('invalid operator identity signature');
  }
  return { operator: Object.freeze(operator), client };
}
