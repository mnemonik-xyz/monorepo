#!/usr/bin/env node
// Installs built source candidates, never publishes or accesses an existing identity.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(process.argv[2] ?? join(root, 'target/protocol-package-drill'));
mkdirSync(output, { recursive: true });
const install = mkdtempSync(join(tmpdir(), 'mnemonik-candidate-install-'));
const run = (command, args, options = {}) => execFileSync(command, args, {
  cwd: root, encoding: 'utf8', timeout: 180000, maxBuffer: 4 * 1024 * 1024,
  stdio: ['ignore', 'pipe', 'pipe'], ...options,
});
const report = { version: 1, kind: 'unpublished_source_candidate',
  checkout_revision: run('git', ['rev-parse', 'HEAD']).trim(),
  build_provenance: 'prebuilt_output_not_rebuilt_by_this_probe',
  status: 'running', started_at: new Date().toISOString(), packages: [], checks: [],
  limits: ['Requires previously built SDK and CLI output; does not establish the build provenance of that output.',
    'npm dependency installation uses the registry; identity/save/open commands reject HTTP fetch.',
    'Local sealed save/open only: no hosted delivery, paid operation or published-release claim.'] };
// Invalidate any older passing observation before replacing same-version tarballs.
writeFileSync(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
try {
  for (const workspace of ['@mnemonik-xyz/sdk', '@mnemonik-xyz/cli']) {
    const [packed] = JSON.parse(run('npm', ['pack', '--ignore-scripts', '--json',
      '--workspace', workspace, '--pack-destination', output]));
    const path = join(output, packed.filename);
    report.packages.push({ name: packed.name, version: packed.version,
      filename: packed.filename, sha256: createHash('sha256').update(readFileSync(path)).digest('hex'),
      npm_integrity: packed.integrity });
  }
  writeFileSync(join(install, 'package.json'), JSON.stringify({ private: true, name: 'mnemonik-candidate-drill', version: '0.0.0' }));
  run('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund',
    ...report.packages.map(pkg => join(output, pkg.filename))], { cwd: install });
  report.checks.push({ name: 'isolated_tarball_install', status: 'passed' });
  for (const pkg of report.packages) {
    const installedPath = realpathSync(join(install, 'node_modules', pkg.name));
    if (!installedPath.startsWith(`${realpathSync(install)}/`)) throw new Error('installed package escapes isolated directory');
    const manifest = JSON.parse(readFileSync(join(installedPath, 'package.json')));
    if (manifest.name !== pkg.name || manifest.version !== pkg.version) throw new Error('installed package differs from candidate manifest');
  }
  const guard = join(install, 'no-network.mjs');
  writeFileSync(guard, `globalThis.fetch = async () => { throw new Error('offline package drill: HTTP forbidden'); };\n`);
  const executable = join(install, 'node_modules/@mnemonik-xyz/cli/dist/bin/mnemonic.js');
  const env = { ...process.env, MNEMONIC_CONFIG_DIR: join(install, 'identity') };
  const cli = args => run(process.execPath, ['--import', guard, executable, '--json', ...args], { cwd: install, env });
  cli(['init', '--standalone']);
  report.checks.push({ name: 'fresh_standalone_identity', status: 'passed' });
  const content = 'Synthetic package recovery fixture';
  const saved = JSON.parse(cli(['sign', content]));
  if (!/^[0-9a-f]{64}$/.test(saved.memoryHash)) throw new Error('save omitted memory hash');
  const opened = JSON.parse(cli(['open', saved.memoryHash]));
  if (opened.content !== content) throw new Error('fresh-process content mismatch');
  report.checks.push({ name: 'offline_sealed_save_and_fresh_process_open', status: 'passed' });
  report.memory_hash = saved.memoryHash;
  report.status = 'passed';
  report.finished_at = new Date().toISOString();
  writeFileSync(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  console.log(`Candidate install and offline recovery passed. Evidence: ${join(output, 'report.json')}`);
} catch (error) {
  report.status = 'failed';
  report.finished_at = new Date().toISOString();
  writeFileSync(join(output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  // Never include command output containing generated identity/artifact material.
  console.error(`Candidate package drill failed: ${error.code ?? error.name ?? 'error'}${error.status ? ` (exit ${error.status})` : ''}`);
  process.exitCode = 1;
} finally {
  rmSync(install, { recursive: true, force: true });
}
