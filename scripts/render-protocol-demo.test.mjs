import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';
import { normalizeReport, renderReport } from './render-protocol-demo.mjs';

const recorded = JSON.parse(readFileSync(new URL('../work/protocol-product/evidence/research-handoff-2026-10-02.json', import.meta.url)));

test('four evidence views show the recorded artifacts and recovery results', () => {
  const dom = new JSDOM(renderReport(recorded), { runScripts: 'dangerously' });
  try {
    const doc = dom.window.document;
    assert.equal(doc.querySelectorAll('[data-artifact]').length, 4);
    doc.querySelector('[data-artifact="S"]').click();
    assert(doc.querySelector('#panel').textContent.includes(recorded.artifacts[1].originalSha256));
    const select = doc.querySelector('#artifact-select');
    select.value = 'W'; select.dispatchEvent(new dom.window.Event('change'));
    assert(doc.querySelector('#panel').textContent.includes(recorded.artifacts[3].artifactId));
    doc.querySelector('[data-tab="failures"]').click();
    for (const failure of recorded.failures) assert(doc.querySelector('#panel').textContent.includes(failure.detail));
    doc.querySelector('[data-tab="recovery"]').click();
    assert(doc.querySelector('#panel').textContent.includes(recorded.recoveredHeads[0]));
    assert.equal(doc.querySelector('[data-tab="recovery"]').getAttribute('aria-selected'), 'true');
  } finally { dom.window.close(); }
});

test('untrusted evidence strings cannot escape JSON or execute markup', () => {
  const input = structuredClone(recorded);
  input.limitations = ['</script><script>globalThis.pwned = true</script><img src=x onerror="globalThis.pwned=true">'];
  input.unexpectedPrivateMaterial = 'must not be serialized';
  const html = renderReport(input);
  assert(!html.includes('must not be serialized'));
  const dom = new JSDOM(html, { runScripts: 'dangerously' });
  try {
    assert.equal(dom.window.pwned, undefined);
    assert.equal(dom.window.document.querySelectorAll('#limits img').length, 0);
    assert(dom.window.document.querySelector('#limits').textContent.includes('</script>'));
  } finally { dom.window.close(); }
});

test('incomplete, inconsistent or secret-bearing declared reports are rejected', () => {
  for (const mutate of [
    r => { r.artifacts[2].prevId = null; },
    r => { delete r.checks.checkpointAuthenticated; },
    r => { r.checks.privateKeysPublished = true; },
    r => { r.environment = 'production'; },
  ]) {
    const input = structuredClone(recorded); mutate(input);
    assert.throws(() => normalizeReport(input));
  }
});
