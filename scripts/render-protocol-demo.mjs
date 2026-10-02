#!/usr/bin/env node
// A local view of recorded assertions, not a simulation of live providers.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const labels = ['R', 'S', 'V', 'W'];
const checks = ['checkpointAuthenticated','manifestAuthenticated','exactOriginalBytes',
  'completeToHeads','restoredIdentity','privateKeysPublished','originalOperatorOffline',
  'originalSourceOffline','operatorPayloadTablesEmpty'];
function string(value, maximum = 512) {
  if (typeof value !== 'string' || !value.length || value.length > maximum) throw new Error('invalid evidence string');
  return value;
}
export function normalizeReport(input) {
  if (input.version !== 1 || input.environment !== 'local_mock_services_real_sdk_wasm') throw new Error('unsupported evidence environment/version');
  if (!Array.isArray(input.artifacts) || input.artifacts.length !== 4 || labels.some(label => input.artifacts.filter(row => row.label === label).length !== 1)) throw new Error('evidence must contain R, S, V and W once each');
  const artifacts = labels.map(label => {
    const row = input.artifacts.find(value => value.label === label);
    for (const key of ['artifactId','author','locator','originalSha256']) string(row[key]);
    if (!/^a2a:[0-9a-f]{64}$/.test(row.artifactId) || !/^[0-9a-f]{64}$/.test(row.originalSha256) ||
        !/^(ar:\/\/[A-Za-z0-9_-]{43}|blob:\/\/[0-9a-f]{64})$/.test(row.locator) ||
        !/^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(row.author) || typeof row.recipientOpened !== 'boolean') throw new Error('invalid artifact evidence');
    if (row.prevId !== null && !/^a2a:[0-9a-f]{64}$/.test(row.prevId)) throw new Error('invalid parent evidence');
    for (const key of ['streamChunks','grants']) if (!Number.isSafeInteger(row[key]) || row[key] < 0) throw new Error('invalid artifact count');
    return Object.fromEntries(['label','artifactId','author','prevId','locator','originalSha256','recipientOpened','streamChunks','grants'].map(key => [key, row[key]]));
  });
  if (artifacts[0].prevId !== null || artifacts.slice(1).some((row, index) => row.prevId !== artifacts[index].artifactId)) throw new Error('R/S/V/W ancestry is inconsistent');
  const observed = {};
  for (const key of checks) {
    if (typeof input.checks?.[key] !== 'boolean') throw new Error(`missing observation: ${key}`);
    observed[key] = input.checks[key];
  }
  if (observed.privateKeysPublished) throw new Error('refusing report that declares published private keys');
  if (!Number.isSafeInteger(input.checks.replacementReceiptCount) || input.checks.replacementReceiptCount < 0) throw new Error('invalid receipt count');
  observed.replacementReceiptCount = input.checks.replacementReceiptCount;
  const operators = { O1: string(input.operators?.O1), O2: string(input.operators?.O2) };
  if (operators.O1 === operators.O2) throw new Error('operator identities must differ');
  if (!Array.isArray(input.recoveredHeads) || input.recoveredHeads.length !== 1 || input.recoveredHeads[0] !== artifacts[2].artifactId) throw new Error('recovered head must be V');
  if (!Array.isArray(input.phases) || input.phases.length > 32 || !Array.isArray(input.failures) || input.failures.length > 32 || !Array.isArray(input.limitations) || input.limitations.length > 32) throw new Error('invalid evidence lists');
  return {version:1, environment:input.environment, artifacts, operators, recoveredHeads:[...input.recoveredHeads], checks:observed,
    phases:input.phases.map(row => { if (!['passed','failed','not_run'].includes(row.status)) throw new Error('invalid phase status'); return {name:string(row.name,160),status:row.status}; }),
    failures:input.failures.map(row => { if (!['rejected','recovered','passed','not_run'].includes(row.status)) throw new Error('invalid failure status'); return {name:string(row.name,160),status:row.status,detail:string(row.detail,1000)}; }),
    limitations:input.limitations.map(value => string(value,1000))};
}

export function renderReport(input) {
  const report = normalizeReport(input);
  // Escape '<' before embedding JSON: even an evidence string containing
  // </script> must never break out of this non-executable data element.
  const data = JSON.stringify(report).replace(/</g, '\\u003c');
  return `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Mnemonic · Research handoff evidence</title>
<style>
:root{color-scheme:dark;font-family:ui-sans-serif,system-ui,sans-serif;background:#0c1421;color:#e7edf4}*{box-sizing:border-box}body{margin:0}main{max-width:1100px;margin:auto;padding:40px 24px}h1{font-size:clamp(30px,5vw,48px);letter-spacing:-.04em;line-height:1.1;margin:18px 0}h2{font-size:23px}p{line-height:1.6;color:#b5c3d4;max-width:780px}.eyebrow{color:#54d2bb;font-size:12px;letter-spacing:.1em;text-transform:uppercase}.notice{padding:16px 20px;background:#162436;border-left:3px solid #54d2bb;border-radius:4px;margin:24px 0}nav{display:flex;gap:8px;flex-wrap:wrap;margin:28px 0}button{font:inherit;color:inherit;background:#142135;border:1px solid #334259;padding:10px 16px;border-radius:8px;cursor:pointer}button[aria-selected=true]{background:#234d4a;border-color:#54d2bb}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:14px}.card{background:#142135;border:1px solid #28384e;border-radius:12px;padding:20px}.card .letter{font-size:32px;font-weight:700;color:#54d2bb}.muted{color:#9bb0c8;font-size:13px}code{font-family:ui-monospace,monospace;font-size:12px;overflow-wrap:anywhere;white-space:normal}dl{display:grid;grid-template-columns:140px 1fr;gap:16px}dt{color:#9bb0c8}dd{margin:0;overflow-wrap:anywhere}.good{color:#74ddc5}.bad{color:#ffb5a8}.row{display:flex;justify-content:space-between;gap:20px;padding:14px 0;border-bottom:1px solid #29384a}.row>span:first-child{min-width:0;overflow-wrap:anywhere}.tag{font-size:12px;border:1px solid #456158;border-radius:100px;padding:4px 8px;white-space:nowrap}select{font:inherit;background:#142135;color:inherit;padding:10px;border:1px solid #456158;border-radius:6px}.limits{margin-top:32px}.limits li{margin:10px 0;color:#b5c3d4;line-height:1.5}footer{margin-top:40px;color:#92a4ba;font-size:12px}a{color:#74ddc5}@media(max-width:520px){dl{grid-template-columns:1fr;gap:6px}dd{margin-bottom:16px}}
</style><main>
<div class="eyebrow">Local mock services · Real SDK and WebAssembly cryptography</div>
<h1>Memory survives the handoff.</h1>
<p>A collector signs research. A reviewer verifies it. After the original service and local state disappear, the reviewer recovers the same memory and continues through another operator.</p>
<div class="notice">This page displays a recorded synthetic drill. It does not contact services, inject failures, or establish live provider availability.</div>
<nav aria-label="Evidence views" role="tablist"><button data-tab="timeline" role="tab">Project timeline</button><button data-tab="artifact" role="tab">Artifact inspection</button><button data-tab="failures" role="tab">Failure checks</button><button data-tab="recovery" role="tab">Recovery status</button></nav>
<section id="panel" role="tabpanel"></section><section class="limits"><h2>What this proves—and its limits</h2><ul id="limits"></ul></section>
<footer>Mnemonic Protocol · Evidence viewer · Private author keys and plaintext are excluded from this report. Signatures do not establish that research claims are true.</footer>
</main><script id="evidence" type="application/json">${data}</script>
<script>
const report=JSON.parse(document.querySelector('#evidence').textContent),panel=document.querySelector('#panel');
const titles={R:'Research scope',S:'Source bundle',V:'Reviewer assessment',W:'Recovered reviewer continuation'};
const esc=value=>String(value).replace(/[&<>"']/g,ch=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
const code=value=>'<code>'+esc(value??'Root: no parent')+'</code>';
const field=(name,value)=>'<dt>'+esc(name)+'</dt><dd>'+value+'</dd>';
let selected='R';
function artifact(){const row=report.artifacts.find(r=>r.label===selected);return '<h2>Inspect a signed artifact</h2><label>Artifact <select id="artifact-select">'+report.artifacts.map(r=>'<option '+(r.label===selected?'selected':'')+'>'+r.label+'</option>').join('')+'</select></label><div class="card" style="margin-top:20px"><h2>'+esc(row.label+' · '+titles[row.label])+'</h2><dl>'+field('Author',code(row.author))+field('Artifact ID',code(row.artifactId))+field('Signed parent',code(row.prevId))+field('Storage locator',code(row.locator))+field('Original SHA-256',code(row.originalSha256))+field('Recipient opened',esc(row.recipientOpened))+field('Stream chunks',esc(row.streamChunks))+field('Embedded grants',esc(row.grants))+'</dl></div>';}
function show(tab){document.querySelectorAll('[data-tab]').forEach(b=>b.setAttribute('aria-selected',String(b.dataset.tab===tab)));if(tab==='timeline')panel.innerHTML='<h2>Four artifacts. Two authors. Two operators.</h2><div class="grid">'+report.artifacts.map(r=>'<article class="card"><div class="letter">'+r.label+'</div><h3>'+titles[r.label]+'</h3><p class="muted">'+(r.label==='R'||r.label==='S'?'Collector A':'Reviewer B')+'</p>'+code(r.artifactId)+'<p><button data-artifact="'+r.label+'">Inspect '+r.label+'</button></p></article>').join('')+'</div><h2>Recorded phases</h2>'+report.phases.map(p=>'<div class="row"><span>'+esc(p.name)+'</span><span class="tag '+(p.status==='passed'?'good':'bad')+'">'+esc(p.status)+'</span></div>').join('');
else if(tab==='artifact')panel.innerHTML=artifact();else if(tab==='failures')panel.innerHTML='<h2>Executed failure checks</h2><p>These outcomes came from assertions in the drill. Other protocol tests and unexecuted live scenarios are not counted here.</p>'+report.failures.map(f=>'<article class="card" style="margin-bottom:12px"><span class="tag">'+esc(f.status)+'</span><h3>'+esc(f.name)+'</h3><p>'+esc(f.detail)+'</p></article>').join('');
else panel.innerHTML='<h2>Recovered through pinned head V</h2><div class="card"><dl>'+field('Expected head',code(report.recoveredHeads[0]))+field('Original O1',code(report.operators.O1))+field('Replacement O2',code(report.operators.O2))+'</dl></div>'+Object.entries(report.checks).map(([name,value])=>'<div class="row"><span>'+esc(name.replace(/([A-Z])/g,' $1'))+'</span><strong>'+esc(value)+'</strong></div>').join('')+'<p>The recovered reviewer uses restored B keys. This is identity continuity, not an independently provisioned new identity. Payment settlement is not exercised by this free local fixture.</p>';
document.querySelectorAll('[data-artifact]').forEach(b=>b.onclick=()=>{selected=b.dataset.artifact;show('artifact')});const select=document.querySelector('#artifact-select');if(select)select.onchange=()=>{selected=select.value;show('artifact')};}
document.querySelectorAll('[data-tab]').forEach(b=>b.onclick=()=>show(b.dataset.tab));document.querySelector('#limits').innerHTML=report.limitations.map(s=>'<li>'+esc(s)+'</li>').join('');show('timeline');
</script></html>`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) throw new Error('Usage: node scripts/render-protocol-demo.mjs report.json report.html');
  writeFileSync(output, renderReport(JSON.parse(readFileSync(input, 'utf8'))));
  console.log(`Evidence view written to ${output}`);
}
