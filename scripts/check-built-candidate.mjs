#!/usr/bin/env node
// Build a committed source snapshot, then exercise installed unpublished tarballs.
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, realpathSync, readdirSync, copyFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join, dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { normalizeReport } from './render-protocol-demo.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const cleanEnv = {...process.env};
for (const key of ['NODE_PATH','NODE_OPTIONS','npm_config_workspace','npm_config_workspaces','RUSTUP_TOOLCHAIN','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER']) delete cleanEnv[key];
const digest = path => createHash('sha256').update(readFileSync(path)).digest('hex');
const query = (command, args, options = {}) => execFileSync(command, args, { cwd:root, encoding:'utf8', timeout:180000, maxBuffer:16*1024*1024, stdio:['ignore','pipe','pipe'], env:cleanEnv, ...options });

export function packageFiles(directory) {
  const files = {};
  function visit(path) {
    for (const entry of readdirSync(path, {withFileTypes:true})) {
      const child = join(path, entry.name);
      if (entry.isSymbolicLink()) throw new Error('package content must not be a workspace symlink');
      if (entry.isDirectory()) visit(child);
      else if (entry.isFile()) files[relative(directory, child)] = digest(child);
    }
  }
  visit(directory);
  return files;
}

async function logged(command, args, {cwd, env, log}) {
  const {openSync,closeSync} = await import('node:fs');
  const fd = openSync(log, 'w');
  try {
    await new Promise((accept,reject) => {
      const child=spawn(command,args,{cwd,env,stdio:['ignore',fd,fd],detached:process.platform!=='win32'});
      let interrupted=false;
      const stop=()=>{interrupted=true;try{if(process.platform!=='win32')process.kill(-child.pid,'SIGKILL');else child.kill('SIGKILL');}catch{}};
      const timer=setTimeout(stop,15*60*1000);
      process.once('SIGINT',stop);process.once('SIGTERM',stop);
      const cleanup=()=>{clearTimeout(timer);process.off('SIGINT',stop);process.off('SIGTERM',stop);};
      child.once('error',error=>{cleanup();reject(error);});
      child.once('exit',(code,signal)=>{cleanup();code===0&&!interrupted?accept():reject(new Error(`${command} failed (${signal??code}); see ${log}`));});
    });
  } finally {closeSync(fd);}
}

async function main() {
  const output=resolve(process.argv[2]??join(root,'target/built-candidate'));
  mkdirSync(output,{recursive:true});
  // This status record is the only success authority; tarballs/logs can remain after failure.
  const report={version:1,kind:'unpublished_clean_source_candidate',status:'running',started_at:new Date().toISOString(),packages:[],checks:[],
    limits:['One clean build with recorded toolchain, not a bit-for-bit reproducibility proof.',
      'Dependency installation and build tools may use the network; artifact/operator services in the drill are local mocks.',
      'SDK and CLI are installed tarballs. MCP is the source test harness, not a deployed or published operator.',
      'No registry publication, live payment, live provider retention, deployed rollback or A-17 release acceptance.']};
  const save=()=>writeFileSync(join(output,'report.json'),`${JSON.stringify(report,null,2)}\n`);
  save();
  const temporary=mkdtempSync(join(tmpdir(),'mnemonik-built-candidate-'));
  try {
    const source=join(temporary,'source'),install=join(temporary,'install');
    mkdirSync(source);mkdirSync(install);
    report.source_revision=query('git',['rev-parse','HEAD']).trim();
    // Runtime source must match the recorded commit; test harness changes have separate hashes.
    const assertRuntimeSource=()=>{
      const dirty=query('git',['diff','HEAD','--','core','mcp/src','mnemonic-a2a','bridge-a2a','Cargo.toml','Cargo.lock','.cargo/config','.cargo/config.toml']);
      if(dirty.trim()||query('git',['rev-parse','HEAD']).trim()!==report.source_revision)throw new Error('runtime source or HEAD changed during candidate validation');
    };
    assertRuntimeSource();
    const archive=join(temporary,'source.tar');
    query('git',['archive','--format=tar',`--output=${archive}`,report.source_revision]);
    report.source_archive_sha256=digest(archive);
    report.source_tree=query('git',['rev-parse',`${report.source_revision}^{tree}`]).trim();
    query('tar',['-xf',archive,'-C',source]);
    report.lockfiles={npm:digest(join(source,'package-lock.json')),cargo:digest(join(source,'Cargo.lock'))};
    report.toolchain={platform:process.platform,architecture:process.arch,node:process.version,npm:query('npm',['--version'],{cwd:source}).trim(),rustc:query('rustc',['-vV'],{cwd:source}).trim(),cargo:query('cargo',['--version'],{cwd:source}).trim(),wasm_pack:query('wasm-pack',['--version'],{cwd:source}).trim()};
    try {report.toolchain.wasm_opt=query('wasm-opt',['--version'],{cwd:source}).trim();} catch {report.toolchain.wasm_opt='not_on_path';}
    const env={...cleanEnv,CARGO_INCREMENTAL:'0',CARGO_TARGET_DIR:join(temporary,'cargo-target')};
    report.commands=[['npm','ci','--ignore-scripts','--no-audit','--no-fund'],['npm','run','build','--workspace','@mnemonik-xyz/sdk'],['npm','run','build','--workspace','@mnemonik-xyz/cli']];
    console.log(`Building clean source ${report.source_revision}; logs in ${output}`);
    await logged('npm',['ci','--ignore-scripts','--no-audit','--no-fund'],{cwd:source,env,log:join(output,'install-build-dependencies.log')});
    await logged('npm',['run','build','--workspace','@mnemonik-xyz/sdk'],{cwd:source,env,log:join(output,'build-sdk.log')});
    await logged('npm',['run','build','--workspace','@mnemonik-xyz/cli'],{cwd:source,env,log:join(output,'build-cli.log')});
    if(report.lockfiles.npm!==digest(join(source,'package-lock.json'))||report.lockfiles.cargo!==digest(join(source,'Cargo.lock')))throw new Error('build changed a source lockfile');
    report.checks.push({name:'clean_source_sdk_wasm_cli_build',status:'passed'});
    for(const workspace of ['@mnemonik-xyz/sdk','@mnemonik-xyz/cli']) {
      const [packed]=JSON.parse(query('npm',['pack','--ignore-scripts','--json','--workspace',workspace,'--pack-destination',output],{cwd:source}));
      report.packages.push({name:packed.name,version:packed.version,filename:packed.filename,sha256:digest(join(output,packed.filename)),npm_integrity:packed.integrity});
    }
    writeFileSync(join(install,'package.json'),JSON.stringify({name:'mnemonik-installed-candidate',version:'0.0.0',private:true}));
    await logged('npm',['install','--ignore-scripts','--no-audit','--no-fund','--workspaces=false',...report.packages.map(pkg=>join(output,pkg.filename))],{cwd:install,env,log:join(output,'install-candidates.log')});
    const installed={};
    const installedLock=JSON.parse(readFileSync(join(install,'package-lock.json')));
    report.installed_lock_sha256=digest(join(install,'package-lock.json'));
    report.installed_dependencies=Object.fromEntries(Object.entries(installedLock.packages).filter(([name])=>name).map(([name,pkg])=>[name,{version:pkg.version,integrity:pkg.integrity??null}]));
    for(const pkg of report.packages) {
      const directory=realpathSync(join(install,'node_modules',pkg.name));
      if(!directory.startsWith(`${realpathSync(install)}/`))throw new Error('installed package escapes disposable installation');
      const manifest=JSON.parse(readFileSync(join(directory,'package.json')));
      if(manifest.name!==pkg.name||manifest.version!==pkg.version)throw new Error('installed candidate identity mismatch');
      const unpacked=join(temporary,`unpacked-${pkg.name.split('/').at(-1)}`);
      mkdirSync(unpacked);
      query('tar',['-xzf',join(output,pkg.filename),'-C',unpacked]);
      const packedFiles=packageFiles(join(unpacked,'package'));
      for(const [file,hash]of Object.entries(packedFiles))if(digest(join(directory,file))!==hash)throw new Error('installed file differs from packed candidate');
      installed[pkg.name]={directory,files:packedFiles};
      pkg.installed_files_sha256=installed[pkg.name].files;
    }
    const sdk=installed['@mnemonik-xyz/sdk'].directory,cli=installed['@mnemonik-xyz/cli'].directory;
    // Resolve the CLI's SDK from inside its installed module, not from the workspace.
    const resolver=join(cli,'candidate-resolution.mjs');
    writeFileSync(resolver,"console.log(import.meta.resolve('@mnemonik-xyz/sdk'));\n");
    const sdkResolution=query(process.execPath,[resolver],{cwd:install}).trim();
    if(realpathSync(fileURLToPath(sdkResolution))!==realpathSync(join(sdk,'dist/index.js')))throw new Error('CLI resolved a different SDK');
    rmSync(resolver);
    report.checks.push({name:'isolated_tarball_install_and_cli_sdk_resolution',status:'passed'});
    // Test harness helpers are copied beside immutable package output so ../dist
    // always resolves to installed bytes. They are not presented as shipped files.
    const harnessNames=['test-migrated-a2a-e2e.mjs','research-crypto-failures.mjs','research-discovery-failures.mjs'];
    report.harness_files_sha256={};
    mkdirSync(join(sdk,'scripts'));mkdirSync(join(sdk,'test/fixtures'),{recursive:true});
    for(const name of harnessNames) {
      const from=join(root,'packages/sdk/scripts',name);
      report.harness_files_sha256[`packages/sdk/scripts/${name}`]=digest(from);
      copyFileSync(from,join(sdk,'scripts',name));
    }
    const fixture='packages/sdk/test/fixtures/sealed-a2a.json';
    report.harness_files_sha256[fixture]=digest(join(root,fixture));
    copyFileSync(join(root,fixture),join(sdk,'test/fixtures/sealed-a2a.json'));
    for(const file of ['mcp/tests/integration_a2a_mcp_tools.rs','mcp/tests/signed_ingestion.rs','scripts/run-protocol-demo.mjs','scripts/render-protocol-demo.mjs','scripts/check-built-candidate.mjs'])report.harness_files_sha256[file]=digest(join(root,file));
    for(const [file,hash]of Object.entries(packageFiles(join(root,'mcp/tests/_helpers'))))report.harness_files_sha256[`mcp/tests/_helpers/${file}`]=hash;
    for(const file of ['.cargo/config','.cargo/config.toml'])if(existsSync(join(root,file)))report.harness_files_sha256[file]=digest(join(root,file));
    const guard=join(install,'no-network.mjs');
    writeFileSync(guard,"globalThis.fetch=async()=>{throw new Error('offline candidate CLI: HTTP forbidden');};\n");
    const cliEnv={...cleanEnv,MNEMONIC_CONFIG_DIR:join(temporary,'identity')};
    const command=args=>query(process.execPath,['--import',guard,join(cli,'dist/bin/mnemonic.js'),'--json',...args],{cwd:install,env:cliEnv});
    command(['init','--standalone']);
    const content='Synthetic clean candidate recovery fixture';
    const saved=JSON.parse(command(['sign',content]));
    if(!/^[0-9a-f]{64}$/.test(saved.memoryHash))throw new Error('CLI did not return memory hash');
    if(JSON.parse(command(['open',saved.memoryHash])).content!==content)throw new Error('installed CLI recovery mismatch');
    report.checks.push({name:'installed_cli_offline_save_fresh_process_open',status:'passed'});
    const demoOutput=join(output,'recovery');
    const demoEnv={...cleanEnv,CARGO_INCREMENTAL:'0',MNEMONIC_DEMO_SDK_SCRIPT:join(sdk,'scripts/test-migrated-a2a-e2e.mjs')};
    assertRuntimeSource();
    console.log('Candidate packages built and installed; running recovery and fault matrix against installed SDK.');
    await logged(process.execPath,[join(root,'scripts/run-protocol-demo.mjs'),demoOutput],{cwd:root,env:demoEnv,log:join(output,'installed-recovery.log')});
    const recovery=normalizeReport(JSON.parse(readFileSync(join(demoOutput,'report.json'))));
    report.recovery={report_sha256:digest(join(demoOutput,'report.json')),controls:recovery.failures.map(row=>row.name),artifact_ids:recovery.artifacts.map(row=>row.artifactId)};
    report.checks.push({name:'installed_sdk_backup_restore_migration_operator_switch_and_failures',status:'passed'});
    // Harness files may have been added, but no shipped file may change during testing.
    for(const pkg of Object.values(installed))for(const [file,hash]of Object.entries(pkg.files))if(digest(join(pkg.directory,file))!==hash)throw new Error('test modified shipped package bytes');
    report.checks.push({name:'shipped_package_files_unchanged_by_drill',status:'passed'});
    assertRuntimeSource();
    for(const [file,hash]of Object.entries(report.harness_files_sha256))if(digest(join(root,file))!==hash)throw new Error('test harness changed during validation');
    report.status='passed';
    console.log(`Clean candidate evidence: ${join(output,'report.json')}`);
  } catch(error) {
    report.status='failed';
    // execFileSync failures can carry command output with generated identities.
    console.error(error.stdout||error.stderr?`Candidate command failed (${error.status??error.code??'error'})`:error.message);
    process.exitCode=1;
  } finally {
    report.finished_at=new Date().toISOString();
    try {save();} finally {rmSync(temporary,{recursive:true,force:true});}
  }
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url))main().catch(error=>{console.error(error.message);process.exitCode=1;});
