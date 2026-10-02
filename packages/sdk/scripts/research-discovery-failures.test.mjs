// Requires the already-built SDK/WASM. No network, package install or build.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {MnemonicClient, Keypair, LocalSigner} from '../dist/index.js';
import {runResearchDiscoveryFailures} from './research-discovery-failures.mjs';

const fixture = JSON.parse(readFileSync(new URL('../test/fixtures/sealed-a2a.json', import.meta.url), 'utf8'));
function local(identity) {
  const key = new Keypair(identity), rows = new Map();
  const client = new MnemonicClient({baseUrl: 'https://unused.invalid', signer: new LocalSigner(key),
    fetch: async () => {throw new Error('fixture construction must stay offline');},
    a2aIndex: {list: async () => [...rows.values()], put: async row => {rows.set(row.attestationId, row);}},
  });
  client.setKeypair(key);
  return {key, rows, client};
}

test('real sealed A/B research ancestry produces four discovery controls without mutating input', async () => {
  const context = 'research-discovery-control-test';
  const a = local(fixture.author), b = local(fixture.reader);
  const recipients = [{card: fixture.card, trustedCardSigner: fixture.reader.pubkey_base58}];
  let previous;
  for (const label of ['R', 'S']) {
    previous = await a.client.attestA2AMessage({...fixture.payload, messageId: label, contextId: context},
      context, {mode: 'local', ...(previous ? {prevId: previous} : {}),
        sealed: {recipients, ...(label === 'S' ? {chunkSize: 19} : {})}});
  }
  for (const row of a.rows.values()) b.rows.set(row.attestationId, row);
  const head = await b.client.attestA2AMessage({...fixture.payload, messageId: 'V', contextId: context},
    context, {mode: 'local', prevId: previous, sealed: {recipients}});
  const rows = [...b.rows.values()], before = JSON.stringify(rows);
  const result = await runResearchDiscoveryFailures({context, rows, identity: b.key, head,
    expectedAuthors: [a.key.pubkey, b.key.pubkey]});
  assert.deepEqual(result.map(row => row.name), ['omitted_required_parent', 'index_outage_vs_empty',
    'index_lag_known_locator_recovery', 'valid_fork_preserved']);
  assert(result.every(row => row.status === 'passed' && row.detail.length > 0));
  assert.equal(JSON.stringify(rows), before);
  assert.equal(b.rows.size, 3);
  // Evidence must never carry the fixture's identity secret or retained envelopes.
  const publicResult = JSON.stringify(result);
  assert(!publicResult.includes(JSON.stringify(fixture.reader.secret)));
  for (const row of rows) assert(!publicResult.includes(row.coseEnvelopeHex));
});
