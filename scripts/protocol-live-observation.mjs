// Pure helpers: caller-supplied submission provenance is not independently attested.
export function validateSubmission(values, pinned, now = Date.now()) {
  const supplied = ['submitted-at', 'submission-evidence'].some(key => values[key] !== undefined);
  if (values['expected-envelope-sha256'] !== undefined &&
      (!pinned || !/^[0-9a-f]{64}$/.test(values['expected-envelope-sha256']))) {
    throw new Error('expected-envelope-sha256 requires fixture pins and 64 lowercase hex characters');
  }
  if (!supplied) return null;
  if (!pinned || !values['submitted-at'] || !values['submission-evidence']) {
    throw new Error('submission observation requires fixture pins, submitted-at and submission-evidence');
  }
  const timestamp = values['submitted-at'];
  const parsed = Date.parse(timestamp);
  if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(timestamp) ||
      !Number.isFinite(parsed) || new Date(parsed).toISOString() !== timestamp || parsed > now) {
    throw new Error('submitted-at must be a valid nonfuture UTC timestamp with milliseconds');
  }
  const reference = values['submission-evidence'];
  if (typeof reference !== 'string' || reference.length > 512 || /[\x00-\x20\x7f]/.test(reference) ||
      !/^[A-Za-z0-9][A-Za-z0-9._:/#-]*$/.test(reference)) {
    throw new Error('submission-evidence must be a public record reference without credentials, query or whitespace');
  }
  return { submitted_at: timestamp, evidence_reference: reference,
    provenance: 'caller_supplied_not_independently_verified' };
}

export function summarizeVisibility(observations, submission) {
  const providers = {};
  for (const provider of ['arweave']) {
    const samples = observations.map(({fixture}) => {
      const index = fixture?.indexes?.[provider];
      if (fixture?.status === 'verified' && index?.expected_locator_present === true) {
        // Both index visibility and independently pinned gateway identity must
        // have completed before we report an observation of this fixture.
        const times = [index.finished_at, fixture.verified_at].map(Date.parse);
        if (times.every(Number.isFinite)) return { state: 'verified_present', at: new Date(Math.max(...times)).toISOString() };
      }
      if (index?.status === 'exhausted' && index.expected_locator_present === false) {
        return { state: 'not_observed_in_exhausted_scan' };
      }
      return { state: 'unknown' }; // Outage, budget, missing or unverified bytes.
    });
    const positive = samples.find(sample => sample.state === 'verified_present');
    const elapsed = positive && submission ? Date.parse(positive.at) - Date.parse(submission.submitted_at) : null;
    providers[provider] = {
      eventually_observed: Boolean(positive),
      first_verified_observation_at: positive?.at ?? null,
      visibility_latency_upper_bound_ms: elapsed !== null && elapsed >= 0 ? elapsed : null,
      initial_sample_positive: samples[0]?.state === 'verified_present',
      sample_states: samples.map(sample => sample.state),
    };
  }
  return { providers, eventually_observed: Object.values(providers).some(value => value.eventually_observed),
    interpretation: 'Upper bounds use caller-supplied submission time and completion of index scan plus gateway verification; not exact ingestion lag. No lower bound or absence claim follows from outage/budget exhaustion. Clock comparability is required.' };
}
