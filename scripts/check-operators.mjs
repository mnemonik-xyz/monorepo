#!/usr/bin/env node
// Read-only identity checks using the built SDK. Never uploads or signs artifacts.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { connectOperator, parseOperatorList } from '../packages/sdk/dist/index.js';

export async function checkOperators(configuration, sessions, fetchImpl = fetch) {
  const list = parseOperatorList(configuration);
  // Bind credentials to both ID and origin before making any request.
  if (!sessions || typeof sessions !== 'object' || Array.isArray(sessions) ||
      Object.keys(sessions).sort().join(',') !== list.operators.map(o => o.id).sort().join(',')) {
    throw new Error('sessions must contain exactly the configured operator IDs');
  }
  for (const operator of list.operators) {
    const session = sessions[operator.id];
    if (session?.baseUrl !== operator.baseUrl || typeof session.jwt !== 'string' || !session.jwt.trim()) {
      throw new Error('each session requires its configured baseUrl and a nonempty jwt');
    }
  }
  const report = { startedAt: new Date().toISOString(), operators: [],
    limitation: 'Read-only key possession checks; storage, recovery and payment readiness are not established.' };
  for (const operator of list.operators) {
    const row = { ...operator, status: 'failed' };
    try {
      await connectOperator(list, operator.id, {
        signer: { pubkey: 'unused-read-only', sign: async () => { throw new Error('read-only probe cannot sign'); } },
        jwt: sessions[operator.id].jwt,
        fetch: (input, init) => fetchImpl(input, { ...init, signal: AbortSignal.timeout(15000) }),
      });
      row.status = 'verified';
    } catch {
      // Server/transport errors may echo credentials. Do not serialize them.
      row.reason = 'Identity check failed; check session validity, endpoint availability and independent key pin.';
    }
    report.operators.push(row);
  }
  report.finishedAt = new Date().toISOString();
  return report;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { values } = parseArgs({ options: { operators: { type: 'string' }, sessions: { type: 'string' } } });
    if (!values.operators || !values.sessions) throw new Error('missing input');
    const report = await checkOperators(JSON.parse(readFileSync(values.operators, 'utf8')),
      JSON.parse(readFileSync(values.sessions, 'utf8')));
    console.log(JSON.stringify(report, null, 2));
    if (report.operators.some(row => row.status !== 'verified')) process.exitCode = 1;
  } catch {
    console.error('Operator probe requires valid --operators and --sessions JSON files. See docs/operator-selection.md.');
    process.exitCode = 1;
  }
}
