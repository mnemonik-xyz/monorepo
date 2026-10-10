import { describe, expect, it } from 'vitest';
import { getPublicKeyAsync, signAsync } from '@noble/ed25519';
import { connectOperator, parseOperatorList } from '../src/operators.js';
import bs58 from './helpers/bs58.js';

const secrets = [new Uint8Array(32).fill(7), new Uint8Array(32).fill(9)];
async function list() {
  return parseOperatorList({ version: 1, operators: await Promise.all(secrets.map(async (key, i) => ({
    id: `o${i + 1}`, baseUrl: `https://o${i + 1}.example.com`,
    publicKey: bs58.encode(await getPublicKeyAsync(key)),
  }))) });
}
const signer = { pubkey: 'unused-for-read-only-proof', sign: async () => { throw new Error('must not sign'); } };
const result = (data: unknown) => new Response(JSON.stringify({ jsonrpc: '2.0', id: 1,
  result: { content: [{ type: 'text', text: JSON.stringify(data) }] } }));
const message = (origin: string, nonce: string) => new TextEncoder().encode(
  `mnemonic.operator-selection.v1\n${origin}\n${nonce}`);
async function proof(config: Awaited<ReturnType<typeof list>>, i: number, nonce: string,
  fault: { key?: number; origin?: string; nonce?: string; signature?: string } = {}) {
  const origin = fault.origin ?? config.operators[i]!.baseUrl;
  const signedNonce = fault.nonce ?? nonce;
  return result({ public_key: config.operators[fault.key ?? i]!.publicKey, origin, nonce: signedNonce,
    algorithm: 'Ed25519', signature: fault.signature ??
      Buffer.from(await signAsync(message(origin, signedNonce), secrets[i]!)).toString('hex') });
}

describe('configured operator selection', () => {
  it('selects explicitly, verifies real signatures, and keeps clients and tokens separate', async () => {
    const config = await list();
    const calls: { host: string; token: string | null; tool: string }[] = [];
    const mock: typeof fetch = async (input, init) => {
      const host = new URL(String(input)).hostname;
      const i = host.startsWith('o1.') ? 0 : 1;
      expect(init?.redirect).toBe('error');
      const body = JSON.parse(String(init?.body));
      calls.push({ host, token: new Headers(init?.headers).get('authorization'), tool: body.params.name });
      if (body.params.name === 'mnemonic_operator_proof') {
        expect(init?.credentials).toBe('omit');
        return proof(config, i, body.params.arguments.nonce);
      }
      return result({ server_pubkey: config.operators[i]!.publicKey });
    };
    const o1 = await connectOperator(config, 'o1', { signer, jwt: 'token-one', fetch: mock });
    const o2 = await connectOperator(config, 'o2', { signer, jwt: 'token-two', fetch: mock });
    expect((await o1.client.whoami()).serverPubkey).toBe(config.operators[0]!.publicKey);
    expect((await o2.client.whoami()).serverPubkey).toBe(config.operators[1]!.publicKey);
    // Identity proofs carry no credential; each token goes only to its operator.
    expect(calls.map(c => [c.host, c.token, c.tool])).toEqual([
      ['o1.example.com', null, 'mnemonic_operator_proof'], ['o2.example.com', null, 'mnemonic_operator_proof'],
      ['o1.example.com', 'Bearer token-one', 'mnemonic_whoami'], ['o2.example.com', 'Bearer token-two', 'mnemonic_whoami'],
    ]);
    expect(Object.isFrozen(o1.operator)).toBe(true);
  });

  it('rejects wrong keys, fabricated signatures, replayed nonces and other origins', async () => {
    const config = await list();
    const faults = [{ key: 1 }, { signature: '00'.repeat(64) }, { nonce: 'ab'.repeat(32) },
      { origin: 'https://o2.example.com' }];
    for (const fault of faults) {
      const mock: typeof fetch = async (_input, init) =>
        proof(config, 0, JSON.parse(String(init?.body)).params.arguments.nonce, fault);
      await expect(connectOperator(config, 'o1', { signer, fetch: mock })).rejects.toThrow(/identity/);
    }
  });

  it('sends no credential and never refreshes a token before the proof succeeds', async () => {
    const config = await list();
    let refreshes = 0;
    const seen: (string | null)[] = [];
    const mock: typeof fetch = async (_input, init) => {
      seen.push(new Headers(init?.headers).get('authorization'));
      return proof(config, 0, JSON.parse(String(init?.body)).params.arguments.nonce, { key: 1 });
    };
    await expect(connectOperator(config, 'o1', { signer, jwt: 'secret-token', fetch: mock,
      tokenRefresher: async () => { refreshes++; return 'fresh-token'; } })).rejects.toThrow(/identity/);
    expect(seen).toEqual([null]);
    expect(refreshes).toBe(0);
  });

  it('bounds the identity proof with a timeout and honors a caller signal', async () => {
    const config = await list();
    const hang: typeof fetch = (_input, init) => new Promise((_resolve, reject) => {
      init?.signal?.addEventListener('abort', () => reject(init.signal!.reason));
    });
    await expect(connectOperator(config, 'o1', { signer, fetch: hang }, { proofTimeoutMs: 20 }))
      .rejects.toThrow(/proof request failed/);
    const controller = new AbortController();
    const pending = connectOperator(config, 'o1', { signer, fetch: hang }, { signal: controller.signal });
    controller.abort();
    await expect(pending).rejects.toThrow(/proof request failed/);
    await expect(connectOperator(config, 'o1', { signer, fetch: hang }, { proofTimeoutMs: 0 }))
      .rejects.toThrow(/proofTimeoutMs/);
  });

  it('never falls back to O2 when O1 is unavailable', async () => {
    const calls: string[] = [];
    const mock: typeof fetch = async input => { calls.push(String(input)); throw new Error('offline'); };
    await expect(connectOperator(await list(), 'o1', { signer, fetch: mock })).rejects.toThrow();
    expect(calls).toEqual(['https://o1.example.com/mcp']);
  });

  it('does not retry an uncertain write on either operator', async () => {
    const config = await list();
    let writes = 0;
    const mock: typeof fetch = async (input, init) => {
      expect(new URL(String(input)).hostname).toBe('o1.example.com');
      const body = JSON.parse(String(init?.body));
      if (body.params.name === 'mnemonic_operator_proof') return proof(config, 0, body.params.arguments.nonce);
      writes++;
      throw new Error('response lost after submission');
    };
    const { client } = await connectOperator(config, 'o1', { signer, fetch: mock,
      keypairProvider: async () => { throw new Error('response failed before signing'); } });
    await expect(client.signMemory('synthetic public test', { mode: 'anchored' })).rejects.toThrow();
    expect(writes).toBe(1);
  });

  it('rejects ambiguous, unsafe and incomplete configuration before making requests', async () => {
    const config = await list();
    for (const baseUrl of ['http://o1.example.com', 'https://o1.example.com/mcp',
      'https://user:password@o1.example.com', 'https://o1.example.com?token=secret']) {
      expect(() => parseOperatorList({ ...config, operators: [{ ...config.operators[0], baseUrl }] })).toThrow();
    }
    for (const field of ['id', 'baseUrl', 'publicKey'] as const) {
      const copy = structuredClone(config);
      copy.operators[1]![field] = copy.operators[0]![field];
      expect(() => parseOperatorList(copy)).toThrow(/distinct/);
    }
    expect(() => parseOperatorList({ ...config, default: 'o1' })).toThrow();
    await expect(connectOperator(config, 'missing', { signer, fetch: async () => {
      throw new Error('must not fetch');
    } })).rejects.toThrow(/not configured/);
  });
});
