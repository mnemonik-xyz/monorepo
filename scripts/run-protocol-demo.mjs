#!/usr/bin/env node
// Sequential local fault drills; this never configures or pays a live provider.
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, renameSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeReport, renderReport } from './render-protocol-demo.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export const requiredFailureNames = [
  'changed_artifact_bytes', 'ciphertext_repaired_public_hashes', 'forged_index_tags', 'outsider_fetch_open',
  'omitted_required_parent', 'index_outage_vs_empty', 'index_lag_known_locator_recovery', 'valid_fork_preserved',
  'receipt_database_read_only', 'settled_upload_failure',
];

export function combineEvidence(handoff, financial) {
  const report = normalizeReport(handoff);
  if (!report.phases.length || report.phases.some(row => row.status !== 'passed') ||
      Object.entries(report.checks).some(([name, value]) => name === 'privateKeysPublished' ? value !== false : name === 'replacementReceiptCount' ? value !== 1 : value !== true) ||
      report.artifacts.some(row => !row.recipientOpened)) {
    throw new Error('handoff assertions did not all pass');
  }
  if (!Array.isArray(financial) || financial.length !== 2) throw new Error('both financial observations are required');
  const scenarios = new Set();
  const billing = [];
  for (const observation of financial) {
    if (observation.version !== 1 || observation.environment !== 'local_mock_services' ||
        !['receipt_failure', 'settled_delivery_failure'].includes(observation.scenario) || scenarios.has(observation.scenario)) {
      throw new Error('invalid financial observation provenance');
    }
    scenarios.add(observation.scenario);
    if (!Array.isArray(observation.failures) || !observation.failures.length || observation.failures.length > 8 ||
        !Array.isArray(observation.billing) || !observation.billing.length || observation.billing.length > 8) {
      throw new Error('financial observations are incomplete');
    }
    report.failures.push(...observation.failures);
    billing.push(...observation.billing);
  }
  report.billing = billing;
  report.limitations.push('Financial controls use a seeded accepted mock-provider receipt, not a live charge or refund. They run separately from the unpaid R/S/V/W handoff.');
  const combined = normalizeReport(report);
  if (new Set(combined.failures.map(row => row.name)).size !== combined.failures.length) throw new Error('duplicate failure observations');
  for (const name of requiredFailureNames) {
    const row = combined.failures.find(value => value.name === name);
    if (!row || row.status === 'not_run') throw new Error(`required failure control was not observed: ${name}`);
  }
  return combined;
}

async function runCargo(args, env) {
  await new Promise((accept, reject) => {
    const child = spawn('cargo', args, { cwd: root, env, stdio: 'inherit' });
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? accept() : reject(new Error(`Cargo drill failed (${signal ?? code})`)));
  });
}

async function main() {
  const output = resolve(process.argv[2] ?? join(root, 'target/protocol-demo'));
  mkdirSync(output, { recursive: true });
  // Compilation or runtime failure must not leave the previous success visible.
  for (const name of ['report.json', 'report.html']) rmSync(join(output, name), { force: true });
  const run = mkdtempSync(join(output, 'run-'));
  const manifest = { version: 1, status: 'running', environment: 'local_mock_services_real_sdk_wasm', startedAt: new Date().toISOString(), runDirectory: run };
  const saveStatus = () => writeFileSync(join(output, 'run-status.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  saveStatus();
  try {
    const env = { ...process.env, CARGO_INCREMENTAL: '0', MNEMONIC_DEMO_EVIDENCE_DIR: join(run, 'handoff'), MNEMONIC_DEMO_FINANCIAL_EVIDENCE_DIR: join(run, 'financial') };
    const common = ['test', '-p', 'mnemonic-mcp', '--features', 'test-support', '--test'];
    console.log('Running the research handoff and cryptography/discovery fault controls. Requires prebuilt SDK/WASM.');
    await runCargo([...common, 'integration_a2a_mcp_tools', 'sdk_migrated_sealed_stream_recipient_continues_through_independent_operator', '--', '--ignored', '--nocapture'], env);
    console.log('Running local receipt and payment-state controls with mock provider evidence.');
    for (const name of ['exact_delivery_retry_and_receipt_failure_are_independent', 'upload_crash_reuses_known_locator_and_terminal_paid_failure_exposes_remedy']) {
      await runCargo([...common, 'signed_ingestion', name, '--', '--exact', '--nocapture'], env);
    }
    const read = path => JSON.parse(readFileSync(path, 'utf8'));
    const combined = combineEvidence(read(join(run, 'handoff/report.json')), ['receipt-failure.json', 'payment-failure.json'].map(name => read(join(run, 'financial', name))));
    writeFileSync(join(run, 'report.json'), `${JSON.stringify(combined, null, 2)}\n`);
    writeFileSync(join(run, 'report.html'), renderReport(combined));
    renameSync(join(run, 'report.json'), join(output, 'report.json'));
    renameSync(join(run, 'report.html'), join(output, 'report.html'));
    manifest.status = 'passed';
    console.log(`Local evidence ready: ${join(output, 'report.html')}`);
  } catch (error) {
    manifest.status = 'failed';
    for (const name of ['report.json', 'report.html']) rmSync(join(output, name), { force: true });
    throw error;
  } finally {
    manifest.finishedAt = new Date().toISOString();
    saveStatus();
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
