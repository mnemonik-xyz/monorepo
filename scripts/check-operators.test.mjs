import { test } from 'node:test';
import assert from 'node:assert/strict';
import { signAsync } from '@noble/ed25519';
import { checkOperators } from './check-operators.mjs';

const seed = new Uint8Array(32).fill(7);
const publicKey = 'GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB';
const origin = 'https://o1.example.com';
const configuration = { version: 1, operators: [{ id: 'o1', baseUrl: origin, publicKey }] };

async function proofFor(nonce, signedOrigin = origin) {
  const message = new TextEncoder().encode(`mnemonic.operator-selection.v1\n${signedOrigin}\n${nonce}`);
  const signature = Buffer.from(await signAsync(message, seed)).toString('hex');
  return new Response(JSON.stringify({ jsonrpc: '2.0', id: 1,
    result: { public_key: publicKey, origin: signedOrigin, nonce, signature } }));
}

test('probe requests one credential-free proof and verifies the real signature', async () => {
  let calls = 0;
  const report = await checkOperators(configuration, async (url, init) => {
    calls++;
    assert.equal(url, `${origin}/mcp`);
    assert.equal(init.redirect, 'error');
    assert.equal(init.credentials, 'omit');
    assert.equal(init.headers.Authorization, undefined);
    assert(init.signal instanceof AbortSignal);
    const body = JSON.parse(init.body);
    assert.equal(body.params.name, 'mnemonic_operator_proof');
    assert.match(body.params.arguments.nonce, /^[0-9a-f]{64}$/);
    return proofFor(body.params.arguments.nonce);
  });
  assert.equal(calls, 1);
  assert.equal(report.operators[0].status, 'verified');
});

test('a proof bound to another origin fails', async () => {
  const report = await checkOperators(configuration, async (_url, init) =>
    proofFor(JSON.parse(init.body).params.arguments.nonce, 'https://other.example.com'));
  assert.equal(report.operators[0].status, 'failed');
});

test('failure reports do not include server or transport error contents', async () => {
  const report = await checkOperators(configuration, async () => {
    throw new Error('server echoed secret-detail');
  });
  assert.equal(report.operators[0].status, 'failed');
  assert(!JSON.stringify(report).includes('secret-detail'));
});
