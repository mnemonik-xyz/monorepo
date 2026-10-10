#!/usr/bin/env node
// Read-only provider observations. No credentials, keys, uploads or payments.
import { parseArgs } from 'node:util';
import { setTimeout as sleep } from 'node:timers/promises';
import { createHash } from 'node:crypto';
import { validateSubmission, summarizeVisibility } from './protocol-live-observation.mjs';
import {
  ArweaveDiscoverySource, ArweaveStorageAdapter,
  verifyA2AAttestation,
} from '../packages/sdk/dist/index.js';

const { values } = parseArgs({ options: {
  arweave: { type: 'string', default: 'https://arweave.net/graphql' },
  gateway: { type: 'string', default: 'https://arweave.net' },
  author: { type: 'string' }, context: { type: 'string' },
  head: { type: 'string' }, locator: { type: 'string' },
  'submitted-at': { type: 'string' }, 'submission-evidence': { type: 'string' },
  'expected-envelope-sha256': { type: 'string' },
  samples: { type: 'string', default: '2' },
  'interval-ms': { type: 'string', default: '1000' },
} });
const samples = Number(values.samples), interval = Number(values['interval-ms']);
if (!Number.isInteger(samples) || samples < 1 || samples > 6 ||
    !Number.isInteger(interval) || interval < 0 || interval > 60000) {
  throw new Error('samples must be 1–6 and interval-ms 0–60000');
}
for (const key of ['arweave', 'gateway']) {
  const url = new URL(values[key]);
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash) {
    throw new Error(`${key} requires an HTTPS URL without credentials, query or fragment`);
  }
}
const pins = ['author', 'context', 'head', 'locator'];
const pinned = pins.every(key => values[key]);
if (pins.some(key => values[key]) && !pinned) throw new Error('supply all four independent fixture pins: author, context, head and locator');
if (pinned && (!/^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(values.author) ||
  !/^a2a:[0-9a-f]{64}$/.test(values.head) || !/^ar:\/\/[A-Za-z0-9_-]{43}$/.test(values.locator) ||
  values.context.length > 1024)) throw new Error('invalid fixture pins');

const submission = validateSubmission(values, pinned);

async function jsonRequest(url, query, variables) {
  const response = await fetch(url, { method: 'POST', redirect: 'error', credentials: 'omit',
    signal: AbortSignal.timeout(15000), headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ query, variables }) });
  if (!response.ok) { await response.body?.cancel(); throw new Error(`HTTP ${response.status}`); }
  const reader = response.body.getReader(), chunks = []; let size = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read(); if (done) break;
      size += value.length; if (size > 1048576) throw new Error('response exceeds 1 MiB');
      chunks.push(value);
    }
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
  const json = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  if (json.errors?.length) throw new Error('GraphQL errors; schema/query not accepted');
  return json;
}

async function inventory(endpoint, kind) {
  // Bounded public inventory, not an assertion about all history or authors.
  const tags = [{ name: 'App-Name', values: ['mnemonic-protocol'] }];
  if (kind) tags.push({ name: 'Mnemonic-Type', values: [kind] });
  const query = `query($tags:[TagFilter!]!){transactions(first:100,tags:$tags,sort:HEIGHT_ASC){edges{node{id}}pageInfo{hasNextPage}}}`;
  const start = performance.now();
  try {
    const result = await jsonRequest(endpoint, query, { tags });
    const tx = result.data?.transactions;
    if (!Array.isArray(tx?.edges) || typeof tx.pageInfo?.hasNextPage !== 'boolean' ||
        tx.edges.some(edge => typeof edge?.node?.id !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(edge.node.id))) throw new Error('malformed index result');
    return { status: tx.pageInfo.hasNextPage ? 'budget_exhausted' : 'exhausted',
      candidates: tx.edges.length, has_more: tx.pageInfo.hasNextPage,
      sample_index_ids: tx.edges.slice(0, 3).map(edge => edge.node.id),
      current_ar_locator_shape: tx.edges.filter(edge => /^[A-Za-z0-9_-]{43}$/.test(edge.node.id)).length,
      other_id_shapes: tx.edges.filter(edge => !/^[A-Za-z0-9_-]{43}$/.test(edge.node.id)).length,
      elapsed_ms: Math.round(performance.now() - start) };
  } catch (error) { return { status: 'unavailable', error: error.message.slice(0, 200), elapsed_ms: Math.round(performance.now() - start) }; }
}

async function fixtureObservation() {
  if (!pinned) return { status: 'not_run', reason: 'independently pinned synthetic A2A fixture required' };
  const scope = { artifactKind: 'a2a', context: values.context, expectedAuthors: [values.author] };
  const observations = {};
  for (const [name, source] of [['arweave', new ArweaveDiscoverySource(values.arweave)]]) {
    const queryStartedAt = new Date().toISOString();
    let cursor, pages = 0, found = false; const seen = new Set();
    try {
      do {
        const page = await source.page(scope, cursor); pages++;
        found ||= page.candidates.some(candidate => candidate.locator === values.locator);
        cursor = page.nextCursor;
        if (cursor && seen.has(cursor)) throw new Error('cursor loop');
        if (cursor) seen.add(cursor);
      } while (cursor && pages < 3);
      observations[name] = { status: cursor ? 'budget_exhausted' : 'exhausted', pages, expected_locator_present: found, started_at: queryStartedAt, finished_at: new Date().toISOString() };
    } catch (error) { observations[name] = { status: 'unavailable', pages, started_at: queryStartedAt, finished_at: new Date().toISOString(), error: error.message.slice(0, 200) }; }
  }
  try {
    const adapter = new ArweaveStorageAdapter(values.gateway);
    const bytes = await adapter.fetch(values.locator);
    const envelopeDigest = createHash('sha256').update(bytes).digest('hex');
    if (values['expected-envelope-sha256'] && envelopeDigest !== values['expected-envelope-sha256']) throw new Error('fixture envelope digest mismatch');
    const verified = await verifyA2AAttestation(Buffer.from(bytes).toString('hex'), values.author);
    if (`a2a:${verified.content_hash}` !== values.head || verified.binding.context_id !== values.context) throw new Error('fixture identity/context mismatch');
    return { status: 'verified', indexes: observations, bytes: bytes.length,
      envelope_sha256: envelopeDigest, verified_at: new Date().toISOString(),
      exact_envelope_pin_checked: Boolean(values['expected-envelope-sha256']),
      artifact_id: values.head, author: verified.signer, sealed: verified.binding.sealed };
  } catch (error) { return { status: 'unverified', indexes: observations, error: error.message.slice(0, 200) }; }
}

const report = { version: 1, started_at: new Date().toISOString(),
  endpoints: { arweave: values.arweave, gateway: values.gateway },
  mode: 'read_only_no_credentials', pinned_fixture: pinned ? Object.fromEntries(pins.map(key => [key, values[key]])) : null,
  submission, expected_envelope_sha256: values['expected-envelope-sha256'] ?? null,
  observations: [], limits: ['Inventory capped at one page of 100; scoped fixture scans capped at three pages.',
    'Public inventory IDs are untrusted hints; non-current locator shapes are counted, not silently converted.',
    'No artifact was uploaded, no payment was submitted, no private content is emitted.',
    'Repeated visibility is not ingestion lag: a separately recorded submission time is required.',
    'No claim of newest heads, full history, sustained availability, recipient decryption or Solana memo parity.'] };
for (let sample = 0; sample < samples; sample++) {
  if (sample) await sleep(interval);
  const startedAt = new Date().toISOString();
  const [arweave, a2aArweave, fixture] = await Promise.all([
    inventory(values.arweave), inventory(values.arweave, 'a2a'), fixtureObservation(),
  ]);
  report.observations.push({ started_at: startedAt, finished_at: new Date().toISOString(),
    inventory: { arweave, a2a_arweave: a2aArweave }, fixture });
}
report.finished_at = new Date().toISOString();
report.positive_fixture_observed = pinned && report.observations.every(({ fixture }) =>
  fixture.status === 'verified' && Object.values(fixture.indexes).some(index => index.expected_locator_present));
report.visibility = summarizeVisibility(report.observations, submission);
report.release_gate = 'not_established';
console.log(JSON.stringify(report, null, 2));
process.exitCode = report.positive_fixture_observed ? 0 : 2;
