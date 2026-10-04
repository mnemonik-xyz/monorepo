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
      if (body.params.name === 'mnemonic_prove_identity') {
        const challenge = body.params.arguments.challenge;
        return result({ public_key: config.operators[i]!.publicKey, challenge, algorithm: 'Ed25519',
          signature: Buffer.from(await signAsync(new TextEncoder().encode(challenge), secrets[i]!)).toString('hex') });
      }
      return result({ server_pubkey: config.operators[i]!.publicKey });
    };
    const o1 = await connectOperator(config, 'o1', { signer, jwt: 'token-one', fetch: mock });
    const o2 = await connectOperator(config, 'o2', { signer, jwt: 'token-two', fetch: mock });
    expect((await o1.client.whoami()).serverPubkey).toBe(config.operators[0]!.publicKey);
    expect((await o2.client.whoami()).serverPubkey).toBe(config.operators[1]!.publicKey);
    expect(calls.map(c => [c.host, c.token])).toEqual([
      ['o1.example.com', 'Bearer token-one'], ['o2.example.com', 'Bearer token-two'],
      ['o1.example.com', 'Bearer token-one'], ['o2.example.com', 'Bearer token-two'],
    ]);
    expect(Object.isFrozen(o1.operator)).toBe(true);
  });

  it('rejects wrong keys, fabricated signatures and replayed challenges', async () => {
    const config = await list();
    for (const fault of ['key', 'signature', 'replay']) {
      const mock: typeof fetch = async (_input, init) => {
        const challenge = JSON.parse(String(init?.body)).params.arguments.challenge;
        const signed = fault === 'replay' ? 'old-challenge' : challenge;
        return result({ public_key: config.operators[fault === 'key' ? 1 : 0]!.publicKey,
          challenge: signed,
          signature: fault === 'signature' ? '00'.repeat(64) :
            Buffer.from(await signAsync(new TextEncoder().encode(signed), secrets[0]!)).toString('hex') });
      };
      await expect(connectOperator(config, 'o1', { signer, fetch: mock })).rejects.toThrow(/identity/);
    }
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
      if (body.params.name === 'mnemonic_prove_identity') {
        const challenge = body.params.arguments.challenge;
        return result({ public_key: config.operators[0]!.publicKey, challenge,
          signature: Buffer.from(await signAsync(new TextEncoder().encode(challenge), secrets[0]!)).toString('hex') });
      }
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
