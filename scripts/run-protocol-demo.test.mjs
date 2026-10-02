import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { JSDOM } from 'jsdom';
import { combineEvidence, requiredFailureNames } from './run-protocol-demo.mjs';
import { renderReport } from './render-protocol-demo.mjs';

const handoff = JSON.parse(readFileSync(new URL('../work/protocol-product/evidence/research-handoff-2026-10-02.json', import.meta.url)));
// Synthetic renderer/composer inputs: these do not claim execution of the drill.
handoff.failures = requiredFailureNames.slice(0, 8).map(name => ({name,status:'passed',detail:'Synthetic report fixture for composer tests.'}));
const financial = () => ['receipt_failure', 'settled_delivery_failure'].map(scenario => ({
  version: 1, environment: 'local_mock_services', scenario,
  failures: [{ name: scenario === 'receipt_failure' ? 'receipt_database_read_only' : 'settled_upload_failure', status: 'passed', detail: 'Synthetic report fixture for viewer tests.' }],
  billing: [{ scenario, paymentStatus: scenario === 'receipt_failure' ? 'none' : 'settled',
    deliveryStatus: scenario === 'receipt_failure' ? 'verified' : 'failed', receiptPersisted: false,
    detail: '</script><script>globalThis.pwned=true</script>' }],
  privateMaterial: 'must not be rendered',
}));

test('combines separate observations without equating payment and delivery or rendering extra fields', () => {
  const combined = combineEvidence(handoff, financial());
  assert.equal(combined.billing.length, 2);
  assert.equal(combined.billing[1].paymentStatus, 'settled');
  assert.equal(combined.billing[1].deliveryStatus, 'failed');
  const html = renderReport(combined);
  assert(!html.includes('must not be rendered'));
  const dom = new JSDOM(html, { runScripts: 'dangerously' });
  try {
    dom.window.document.querySelector('[data-tab="failures"]').click();
    const text = dom.window.document.querySelector('#panel').textContent;
    assert(text.includes('No live settlement'));
    assert(text.includes('settled'));
    assert(text.includes('failed'));
    assert.equal(dom.window.pwned, undefined);
  } finally { dom.window.close(); }
});

test('missing, repeated and live financial reports cannot complete the local combined report', () => {
  assert.throws(() => combineEvidence(handoff, financial().slice(0, 1)));
  assert.throws(() => combineEvidence({...handoff, failures: handoff.failures.slice(1)}, financial()));
  for (const mutate of [r => {r.phases[0].status = 'failed';}, r => {r.checks.completeToHeads = false;}, r => {r.artifacts[0].recipientOpened = false;}]) {
    const failed = structuredClone(handoff); mutate(failed);
    assert.throws(() => combineEvidence(failed, financial()));
  }
  for (const mutate of [
    rows => { rows[1] = rows[0]; },
    rows => { rows[0].environment = 'production'; },
    rows => { rows[0].billing = []; },
    rows => { rows[0].failures = []; },
    rows => { rows[0].billing[0].receiptPersisted = 'yes'; },
    rows => { rows[1].failures[0].name = rows[0].failures[0].name; },
  ]) {
    const rows = financial(); mutate(rows);
    assert.throws(() => combineEvidence(handoff, rows));
  }
});

test('a Cargo startup failure invalidates old JSON and HTML success before running the drill', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'mnemonik-demo-runner-'));
  try {
    const bin = join(temporary, 'bin'), out = join(temporary, 'out');
    mkdirSync(bin); mkdirSync(out);
    writeFileSync(join(bin, 'cargo'), '#!/bin/sh\nexit 42\n', { mode: 0o755 });
    for (const name of ['report.json', 'report.html']) writeFileSync(join(out, name), 'old success');
    const result = spawnSync(process.execPath, [new URL('./run-protocol-demo.mjs', import.meta.url).pathname, out], {
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}` }, encoding: 'utf8', timeout: 15000,
    });
    assert.equal(result.status, 1, result.stderr);
    assert.equal(existsSync(join(out, 'report.json')), false);
    assert.equal(existsSync(join(out, 'report.html')), false);
    assert.equal(JSON.parse(readFileSync(join(out, 'run-status.json'))).status, 'failed');
  } finally { rmSync(temporary, { recursive: true, force: true }); }
});

test('standalone renderer removes an old success when its replacement report is invalid', () => {
  const temporary = mkdtempSync(join(tmpdir(), 'mnemonik-demo-render-'));
  try {
    const input = join(temporary, 'invalid.json'), output = join(temporary, 'report.html');
    writeFileSync(input, '{}'); writeFileSync(output, 'old success');
    const result = spawnSync(process.execPath, [new URL('./render-protocol-demo.mjs', import.meta.url).pathname, input, output], {encoding:'utf8'});
    assert.notEqual(result.status, 0);
    assert.equal(existsSync(output), false);
    assert.equal(readFileSync(input, 'utf8'), '{}');
  } finally {rmSync(temporary, {recursive:true, force:true});}
});
