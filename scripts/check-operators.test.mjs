import { test } from 'node:test';
import assert from 'node:assert/strict';
import { signAsync } from '@noble/ed25519';
import { checkOperators } from './check-operators.mjs';

const seed = new Uint8Array(32).fill(7);
const publicKey = 'GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB';
const configuration = { version: 1, operators: [{ id: 'o1', baseUrl: 'https://o1.example.com', publicKey }] };
const sessions = { o1: { baseUrl: 'https://o1.example.com', jwt: 'private-token' } };

test('probe only requests a fresh identity proof and verifies the real signature', async () => {
  let calls = 0;
  const report = await checkOperators(configuration, sessions, async (url, init) => {
    calls++;
    assert.equal(url, 'https://o1.example.com/mcp');
    assert.equal(init.redirect, 'error');
    assert.equal(init.headers.Authorization, 'Bearer private-token');
    const body = JSON.parse(init.body);
    assert.equal(body.params.name, 'mnemonic_prove_identity');
    const challenge = body.params.arguments.challenge;
    assert(challenge.startsWith('mnemonic.operator-selection.v1\nhttps://o1.example.com\n'));
    const signature = Buffer.from(await signAsync(new TextEncoder().encode(challenge), seed)).toString('hex');
    return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1, result: { public_key: publicKey, challenge, signature } }));
  });
  assert.equal(calls, 1);
  assert.equal(report.operators[0].status, 'verified');
  assert(!JSON.stringify(report).includes('private-token'));
});

test('invalid origin binding fails before credentials are transmitted', async () => {
  await assert.rejects(() => checkOperators(configuration, { o1: { ...sessions.o1, baseUrl: 'https://other.example.com' } },
    async () => { assert.fail('must not fetch'); }), /baseUrl/);
});

test('failure reports do not include server or transport error contents', async () => {
  const report = await checkOperators(configuration, sessions, async () => {
    throw new Error('server echoed private-token');
  });
  assert.equal(report.operators[0].status, 'failed');
  assert(!JSON.stringify(report).includes('private-token'));
});
