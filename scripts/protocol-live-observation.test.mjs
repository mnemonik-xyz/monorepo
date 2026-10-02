import test from 'node:test';
import assert from 'node:assert/strict';
import { validateSubmission, summarizeVisibility } from './protocol-live-observation.mjs';
const submitted = '2026-10-02T00:00:00.000Z';
const submission = { submitted_at: submitted };
function sample(status, present, seconds = 2, verified = true) {
  return { fixture: { status: verified ? 'verified' : 'unverified',
    verified_at: `2026-10-02T00:00:0${seconds}.000Z`, indexes: {
      irys: { status, expected_locator_present: present, finished_at: `2026-10-02T00:00:0${seconds-1}.000Z` },
    } } };
}
test('absent then found records conservative completed-verification upper bound', () => {
  const result = summarizeVisibility([sample('exhausted',false),sample('exhausted',true,4)],submission);
  assert.equal(result.eventually_observed,true);
  assert.deepEqual(result.providers.irys.sample_states,['not_observed_in_exhausted_scan','verified_present']);
  assert.equal(result.providers.irys.visibility_latency_upper_bound_ms,4000);
  assert.equal(result.providers.irys.initial_sample_positive,false);
});
test('outage and budget exhaustion remain unknown, then known presence can recover', () => {
  const result = summarizeVisibility([sample('unavailable',undefined),sample('budget_exhausted',false),sample('budget_exhausted',true,5)],submission);
  assert.deepEqual(result.providers.irys.sample_states,['unknown','unknown','verified_present']);
  assert.equal(result.providers.irys.visibility_latency_upper_bound_ms,5000);
  assert.equal(result.providers.arweave.eventually_observed,false);
  assert.equal(result.providers.arweave.visibility_latency_upper_bound_ms,null);
});
test('initial visibility has upper bound only; no submission means no latency value', () => {
  const first = [sample('exhausted',true)];
  assert.equal(summarizeVisibility(first,submission).providers.irys.initial_sample_positive,true);
  assert.equal(summarizeVisibility(first,null).providers.irys.visibility_latency_upper_bound_ms,null);
  assert.equal(summarizeVisibility([sample('exhausted',true,2,false)],submission).eventually_observed,false);
});
test('submission validates full pins, paired provenance, exact UTC, nonfuture clock and digest', () => {
  const values = {'submitted-at':submitted,'submission-evidence':'evidence/submission-2026-10-02.json#upload','expected-envelope-sha256':'a'.repeat(64)};
  const now = Date.parse(submitted)+1000;
  assert.equal(validateSubmission(values,true,now).submitted_at,submitted);
  assert.equal(validateSubmission({},false,now),null);
  assert.throws(()=>validateSubmission(values,false,now));
  assert.throws(()=>validateSubmission(values,true,now-2000));
  for (const patch of [
    {'submission-evidence':undefined}, {'submitted-at':'2026-02-30T00:00:00.000Z'},
    {'submitted-at':'2026-10-02T00:00:00Z'}, {'submission-evidence':'https://user:secret@example.test/log'},
    {'submission-evidence':'https://example.test/log?token=secret'}, {'expected-envelope-sha256':'a'.repeat(63)},
  ]) assert.throws(()=>validateSubmission({...values,...patch},true,now));
});
