// Local failure controls over the demo's original R/S/V signed envelopes.
// Real SDK/WASM verification and signing; synthetic GraphQL and gateway transport.
// No external calls, provider availability claims, or decryption keys in evidence.
import assert from 'node:assert/strict';
import {MnemonicClient, LocalSigner, verifyA2AAttestation} from '../dist/index.js';

const gateway = 'https://research-gateway.invalid';
const index = 'https://research-index.invalid/graphql';
const operator = 'https://research-operator.invalid';
const snapshot = value => JSON.parse(JSON.stringify(value));
const ids = report => report.attestations.map(row => row.attestationId).sort();

/**
 * rows: actual retained R/S/V Attestations; head: independently expected V ID.
 * identity: restored reviewer Keypair; expectedAuthors: independent A/B trust pins.
 * Synthetic locators are deliberately distinct from real migration destinations.
 * Returns public assertion results only; throws if any control fails.
 */
export async function runResearchDiscoveryFailures({context, rows, identity, head, expectedAuthors}) {
  const originals = snapshot(rows);
  const authors = [...expectedAuthors];
  assert.equal(originals.length, 3, 'controls require the three R/S/V envelopes');
  assert(authors.includes(identity.pubkey), 'reviewer must be independently trusted');
  const byId = new Map();
  for (const row of originals) {
    assert(authors.includes(row.signerPubkey), 'row author is outside the trusted set');
    const verified = await verifyA2AAttestation(row.coseEnvelopeHex, row.signerPubkey);
    assert.equal(row.attestationId, `a2a:${verified.content_hash}`);
    assert.equal(verified.binding.context_id, context);
    assert.equal(row.prevId, verified.binding.prev_id);
    assert(!byId.has(row.attestationId));
    byId.set(row.attestationId, row);
  }
  const review = byId.get(head);
  assert(review, 'pinned review head absent');
  assert.equal(review.signerPubkey, identity.pubkey);
  const source = byId.get(review.prevId);
  const root = source && byId.get(source.prevId);
  assert(source && root && !root.prevId, 'expected signed R -> S -> V ancestry');

  const registry = new Map();
  function register(row, letter) {
    const locator = `ar://${letter.repeat(43)}`;
    registry.set(row.attestationId, {...snapshot(row), locator});
    return locator;
  }
  register(root, 'R'); register(source, 'S'); register(review, 'V');
  const expectedIds = [...byId.keys()].sort();
  const options = {expectedAuthors: authors, heads: [head]};
  let calls = 0;
  // Each scenario gets an empty index, unless a test explicitly rescans its own client.
  function scenario(visible, unavailable = false) {
    const state = {visible: [...visible], unavailable};
    const local = new Map();
    const request = async (url, init) => {
      calls++;
      const target = String(url);
      assert(!new Headers(init?.headers).has('authorization'), 'JWT reached discovery/storage');
      assert.notEqual(target.startsWith(operator), true, 'control contacted an operator');
      if (target === index) {
        assert.equal(init?.method, 'POST');
        if (state.unavailable) return new Response('synthetic index outage', {status: 503});
        return Response.json({data: {transactions: {
          edges: state.visible.map(id => {
            const row = registry.get(id);
            assert(row);
            // Tags route candidates only; every envelope is verified against caller-pinned authors.
            const locator = row.locator.slice(5);
            return {cursor: locator, node: {id: locator, tags: Object.entries({
              'App-Name': 'mnemonic-protocol', 'Mnemonic-Type': 'a2a',
              'Producer': row.signerPubkey, 'Context-Id': context,
              'Content-Hash': row.attestationId.slice(4),
            }).map(([name, value]) => ({name, value}))}};
          }), pageInfo: {hasNextPage: false},
        }}});
      }
      assert(target.startsWith(`${gateway}/`), `unexpected network target: ${target}`);
      const row = [...registry.values()].find(item => `${gateway}/${item.locator.slice(5)}` === target);
      return row ? new Response(Buffer.from(row.coseEnvelopeHex, 'hex')) : new Response('', {status: 404});
    };
    const client = new MnemonicClient({baseUrl: operator, signer: new LocalSigner(identity),
      jwt: 'SYNTHETIC-MUST-NOT-LEAK', a2aGatewayUrl: gateway, a2aIndexUrl: index,
      a2aIndexFlavour: 'irys', fetch: request,
      a2aIndex: {list: async () => [...local.values()], put: async row => {local.set(row.attestationId, row);}},
    });
    client.setKeypair(identity);
    return {client, state, local};
  }
  function knownLocators() {
    return Object.fromEntries(originals.map(row => [row.attestationId,
      {author: row.signerPubkey, locator: registry.get(row.attestationId).locator}]));
  }
  function assertExact(report) {
    assert.deepEqual(ids(report), expectedIds);
    assert.equal(report.invalidCandidates.length, 0);
    assert.equal(report.missingParents.length, 0);
    for (const row of report.attestations) assert.equal(row.coseEnvelopeHex, byId.get(row.attestationId).coseEnvelopeHex);
  }
  const evidence = [];

  const omitted = await scenario([root.attestationId, head]).client.restoreA2AContext(context, options);
  assert.equal(omitted.source.status, 'exhausted');
  assert.equal(omitted.scanExhausted, true);
  assert.equal(omitted.completeToHeads, false);
  assert.equal(omitted.completeness, 'unknown');
  assert(omitted.missingParents.includes(source.attestationId));
  assert(omitted.missingParents.includes(head));
  assert.deepEqual(ids(omitted), [root.attestationId]);
  assert.equal(omitted.invalidCandidates.length, 0);
  evidence.push({name: 'omitted_required_parent', status: 'passed',
    detail: `Mock index omitted required S (${source.attestationId}); only R imported. Signed V stayed incomplete despite exhausted discovery.`});

  const empty = await scenario([]).client.restoreA2AContext(context, options);
  const outage = await scenario([], true).client.restoreA2AContext(context, options);
  assert.equal(empty.source.status, 'exhausted'); assert.equal(empty.scanExhausted, true);
  assert.equal(empty.error, undefined); assert.deepEqual(ids(empty), []);
  assert.equal(outage.source.status, 'unavailable'); assert.equal(outage.scanExhausted, false);
  assert(outage.source.error?.includes('503')); assert.deepEqual(ids(outage), []);
  for (const report of [empty, outage]) {
    assert.equal(report.completeToHeads, false); assert.equal(report.completeness, 'unknown');
    assert(report.missingParents.includes(head));
  }
  evidence.push({name: 'index_outage_vs_empty', status: 'passed',
    detail: 'Mock HTTP 503 reported unavailable with an error and no exhausted scan; an empty successful page reported exhausted. Neither completed the pinned head.'});

  const lag = scenario([root.attestationId]);
  const lagged = await lag.client.restoreA2AContext(context, options);
  assert.equal(lagged.source.status, 'exhausted'); assert.equal(lagged.completeToHeads, false);
  assert.deepEqual(ids(lagged), [root.attestationId]);
  lag.state.visible = expectedIds;
  const caughtUp = await lag.client.restoreA2AContext(context, options);
  assertExact(caughtUp); assert.equal(caughtUp.completeToHeads, true);
  // Known locators must work even while discovery remains demonstrably unavailable.
  const bypass = await scenario([], true).client.restoreA2AContext(context,
    {...options, parentLocators: knownLocators()});
  assertExact(bypass); assert.equal(bypass.completeToHeads, true);
  assert.equal(bypass.completeness, 'complete_to_heads');
  assert.equal(bypass.source.status, 'unavailable'); assert.equal(bypass.scanExhausted, false);
  assert(bypass.source.error?.includes('503'));
  const disabled = await scenario([]).client.restoreA2AContext(context,
    {...options, discoverySource: false, parentLocators: knownLocators()});
  assertExact(disabled); assert.equal(disabled.completeToHeads, true);
  assert.equal(disabled.source.status, 'disabled'); assert.equal(disabled.scanExhausted, false);
  evidence.push({name: 'index_lag_known_locator_recovery', status: 'passed',
    detail: 'Mock lag first withheld S/V, then a later scan recovered exact R/S/V. Pinned locators also completed ancestry with discovery unavailable or disabled; source uncertainty remained visible.'});

  // Sign a real competing B child of S. Do not modify original V or caller's index.
  const writer = scenario([]);
  for (const row of registry.values()) writer.local.set(row.attestationId, snapshot(row));
  const beforeSigningCalls = calls;
  const branchId = await writer.client.attestA2AMessage({messageId: 'synthetic-competing-review',
    role: 'agent', contextId: context, parts: [{kind: 'text', text: 'Synthetic alternate review for fork detection only.'}]},
  context, {mode: 'local', prevId: source.attestationId});
  assert.equal(calls, beforeSigningCalls, 'local fork creation used network');
  assert.notEqual(branchId, head);
  const branch = writer.local.get(branchId);
  assert.equal(branch.prevId, source.attestationId);
  register(branch, 'F');
  const forkOptions = {expectedAuthors: authors, heads: [head, branchId]};
  const forked = await scenario([...expectedIds, branchId]).client.restoreA2AContext(context, forkOptions);
  assert.equal(forked.completeToHeads, true); assert.equal(forked.invalidCandidates.length, 0);
  assert.equal(forked.missingParents.length, 0);
  assert.deepEqual(ids(forked), [...expectedIds, branchId].sort());
  const siblings = forked.attestations.filter(row => row.prevId === source.attestationId);
  assert.deepEqual(siblings.map(row => row.attestationId).sort(), [head, branchId].sort());
  for (const row of siblings) assert.equal(row.signerPubkey, identity.pubkey);
  assert.equal(forked.attestations.find(row => row.attestationId === head).coseEnvelopeHex, review.coseEnvelopeHex);
  const unpinned = await scenario([...expectedIds, branchId]).client.restoreA2AContext(context, {expectedAuthors: authors});
  assert.equal(unpinned.completeToHeads, false); assert.equal(unpinned.completeness, 'unknown');
  assert.equal(unpinned.attestations.length, 4);
  evidence.push({name: 'valid_fork_preserved', status: 'passed',
    detail: 'Real SDK/WASM signed a competing reviewer child of S. Restore retained both valid branches and exact V bytes; both pinned heads completed, while no-head recovery remained unknown.'});

  return evidence;
}
