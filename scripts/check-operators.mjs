#!/usr/bin/env node
// Read-only identity checks using the built SDK. Sends no credentials; never uploads or signs.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { parseOperatorList, verifyOperatorIdentity } from '../packages/sdk/dist/index.js';

export async function checkOperators(configuration, fetchImpl = fetch) {
  const list = parseOperatorList(configuration);
  const report = { startedAt: new Date().toISOString(), operators: [],
    limitation: 'Read-only key possession checks; storage, recovery and payment readiness are not established.' };
  for (const operator of list.operators) {
    const row = { ...operator, status: 'failed' };
    try {
      await verifyOperatorIdentity(operator, { fetch: fetchImpl, proofTimeoutMs: 15000 });
      row.status = 'verified';
    } catch {
      // Do not serialize server or transport errors.
      row.reason = 'Identity check failed; check endpoint availability, operator_proof support and independent key pin.';
    }
    report.operators.push(row);
  }
  report.finishedAt = new Date().toISOString();
  return report;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { values } = parseArgs({ options: { operators: { type: 'string' } } });
    if (!values.operators) throw new Error('missing input');
    const report = await checkOperators(JSON.parse(readFileSync(values.operators, 'utf8')));
    console.log(JSON.stringify(report, null, 2));
    if (report.operators.some(row => row.status !== 'verified')) process.exitCode = 1;
  } catch {
    console.error('Operator probe requires a valid --operators JSON file. See docs/operator-selection.md.');
    process.exitCode = 1;
  }
}
