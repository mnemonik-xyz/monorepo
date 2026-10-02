import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,mkdirSync,writeFileSync,readFileSync,rmSync,symlinkSync,readdirSync} from 'node:fs';
import {join} from 'node:path';
import {tmpdir} from 'node:os';
import {spawnSync} from 'node:child_process';
import {packageFiles} from './check-built-candidate.mjs';

test('package manifest detects changed shipped bytes and rejects workspace symlinks',()=>{
  const temporary=mkdtempSync(join(tmpdir(),'mnemonik-package-manifest-'));
  try {
    mkdirSync(join(temporary,'dist'));
    writeFileSync(join(temporary,'dist/index.js'),'export const version=1;');
    const original=packageFiles(temporary);
    assert.deepEqual(Object.keys(original),['dist/index.js']);
    writeFileSync(join(temporary,'dist/index.js'),'export const version=2;');
    assert.notEqual(packageFiles(temporary)['dist/index.js'],original['dist/index.js']);
    symlinkSync(join(temporary,'dist/index.js'),join(temporary,'workspace-link'));
    assert.throws(()=>packageFiles(temporary),/workspace symlink/);
  } finally {rmSync(temporary,{recursive:true,force:true});}
});

test('failed source capture invalidates earlier success and removes disposable build directories',()=>{
  const temporary=mkdtempSync(join(tmpdir(),'mnemonik-candidate-failure-'));
  try {
    const bin=join(temporary,'bin'),output=join(temporary,'output');
    mkdirSync(bin);mkdirSync(output);
    writeFileSync(join(bin,'git'),'#!/bin/sh\nexit 42\n',{mode:0o755});
    writeFileSync(join(output,'report.json'),JSON.stringify({status:'passed'}));
    const result=spawnSync(process.execPath,[new URL('./check-built-candidate.mjs',import.meta.url).pathname,output],{
      env:{...process.env,TMPDIR:temporary,PATH:`${bin}:${process.env.PATH}`},encoding:'utf8',timeout:15000,
    });
    assert.equal(result.status,1,result.stderr);
    assert.equal(JSON.parse(readFileSync(join(output,'report.json'))).status,'failed');
    assert.deepEqual(readdirSync(temporary).sort(),['bin','output']);
  } finally {rmSync(temporary,{recursive:true,force:true});}
});
