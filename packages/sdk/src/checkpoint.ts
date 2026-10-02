/** Owner-authenticated recovery scope. Timestamps never prove newest-head status. */
import { verifyAsync } from '@noble/ed25519';
import { jcsBytes } from './erc8004/jcs.js';
import type { SignerInterface, A2ARestoreOptions } from './types.js';
import { IntegrityError, UserError } from './errors.js';

export interface RecoveryCheckpoint {
  version: 1;
  artifactKind: 'memory' | 'a2a';
  scope: string;
  expectedAuthors: string[];
  heads: string[];
  locators: { artifactId: string; author: string; locator: string; envelopeDigest?: string }[];
  backendHints: string[];
  createdAt: string;
}
export interface SignedRecoveryCheckpoint { checkpoint: RecoveryCheckpoint; signer: string; signature: string }
const DOMAIN = 'mnemonic.recovery-checkpoint.v1\n';
const MAX_BYTES = 1048576;
const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
export function checkpointPublicKey(key: string): Uint8Array {
  if (typeof key !== 'string' || key.length < 32 || key.length > 44) throw new UserError('invalid checkpoint public key');
  let n = 0n;
  for (const c of key) { const digit = alphabet.indexOf(c); if (digit < 0) throw new UserError('invalid checkpoint public key'); n = n * 58n + BigInt(digit); }
  const tail: number[] = []; while (n) { tail.unshift(Number(n & 255n)); n >>= 8n; }
  const leading = key.match(/^1*/)?.[0].length ?? 0;
  const bytes = Uint8Array.from([...new Array<number>(leading).fill(0), ...tail]);
  if (bytes.length !== 32) throw new UserError('checkpoint key must be 32 bytes');
  return bytes;
}
function objectKeys(value: unknown, keys: string[]): asserts value is Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).sort().join('|') !== keys.sort().join('|')) throw new UserError('invalid checkpoint fields');
}
function text(value: unknown): asserts value is string {
  if (typeof value !== 'string' || value.length < 1 || value.length > 1024) throw new UserError('invalid checkpoint string');
}
function strings(value: unknown, max: number): asserts value is string[] {
  if (!Array.isArray(value) || value.length > max) throw new UserError('invalid checkpoint list');
  value.forEach(text); if (new Set(value).size !== value.length) throw new UserError('duplicate checkpoint value');
}
export function checkpointBytes(checkpoint: RecoveryCheckpoint): Uint8Array {
  objectKeys(checkpoint, ['version','artifactKind','scope','expectedAuthors','heads','locators','backendHints','createdAt']);
  if (checkpoint.version !== 1 || !['memory','a2a'].includes(checkpoint.artifactKind)) throw new UserError('unsupported checkpoint version or kind');
  text(checkpoint.scope); strings(checkpoint.expectedAuthors, 256); strings(checkpoint.heads, 10000); strings(checkpoint.backendHints, 32);
  if (!checkpoint.expectedAuthors.length) throw new UserError('checkpoint requires expected authors');
  checkpoint.expectedAuthors.forEach(checkpointPublicKey);
  if (typeof checkpoint.createdAt !== 'string' || !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(checkpoint.createdAt) || !Number.isFinite(Date.parse(checkpoint.createdAt)) || new Date(checkpoint.createdAt).toISOString() !== checkpoint.createdAt) throw new UserError('invalid checkpoint timestamp');
  if (!Array.isArray(checkpoint.locators) || checkpoint.locators.length > 10000) throw new UserError('invalid checkpoint locators');
  const seen = new Set<string>();
  const identities = new Map<string,string>();
  for (const entry of checkpoint.locators) {
    objectKeys(entry, checkpoint.artifactKind === 'memory' ? ['artifactId','author','locator','envelopeDigest'] : ['artifactId','author','locator']);
    if (checkpoint.artifactKind === 'memory' && (typeof entry.envelopeDigest !== 'string' || !/^[0-9a-f]{64}$/.test(entry.envelopeDigest))) throw new UserError('memory checkpoint requires exact envelope digest');
    text(entry.artifactId); text(entry.author); text(entry.locator);
    if (!checkpoint.expectedAuthors.includes(entry.author) || !(/^(?:ar:\/\/[A-Za-z0-9_-]{43}|blob:\/\/[0-9a-f]{64})$/.test(entry.locator))) throw new UserError('invalid checkpoint locator binding');
    const binding = `${entry.author}:${entry.envelopeDigest ?? ''}`;
    if (identities.has(entry.artifactId) && identities.get(entry.artifactId) !== binding) throw new UserError('conflicting checkpoint artifact bindings');
    identities.set(entry.artifactId,binding);
    const identity = `${entry.artifactId}:${entry.locator}`; if (seen.has(identity)) throw new UserError('duplicate checkpoint locator'); seen.add(identity);
  }
  if (checkpoint.artifactKind === 'memory' && checkpoint.heads.some(id => !checkpoint.locators.some(entry => entry.artifactId === id))) throw new UserError('every memory head requires a digest-bound locator');
  const canonical = jcsBytes(checkpoint);
  if (canonical.length > MAX_BYTES) throw new UserError('checkpoint exceeds 1 MiB');
  const prefix = new TextEncoder().encode(DOMAIN), bytes = new Uint8Array(prefix.length + canonical.length);
  bytes.set(prefix); bytes.set(canonical, prefix.length); return bytes;
}
export async function signRecoveryCheckpoint(checkpoint: RecoveryCheckpoint, signer: SignerInterface): Promise<SignedRecoveryCheckpoint> {
  checkpointPublicKey(signer.pubkey);
  // Clone before awaiting: concurrent caller mutation cannot change the signed object.
  const snapshot = JSON.parse(JSON.stringify(checkpoint)) as RecoveryCheckpoint;
  const bytes = checkpointBytes(snapshot), signature = await signer.sign(bytes);
  if (signature.length !== 64) throw new IntegrityError('invalid checkpoint signature size');
  const result = {checkpoint:snapshot, signer:signer.pubkey, signature:Array.from(signature,b=>b.toString(16).padStart(2,'0')).join('')};
  await verifyRecoveryCheckpoint(result, signer.pubkey, {artifactKind:snapshot.artifactKind,scope:snapshot.scope});
  return result;
}
export async function verifyRecoveryCheckpoint(signed: SignedRecoveryCheckpoint, trustedOwner: string, expectedScope: {artifactKind:'memory'|'a2a';scope:string}): Promise<RecoveryCheckpoint> {
  objectKeys(signed, ['checkpoint','signer','signature']);
  signed = JSON.parse(JSON.stringify(signed)) as SignedRecoveryCheckpoint;
  if (signed.signer !== trustedOwner || typeof signed.signature !== 'string' || !/^[0-9a-f]{128}$/.test(signed.signature)) throw new IntegrityError('checkpoint owner or signature mismatch');
  const bytes = checkpointBytes(signed.checkpoint);
  if (signed.checkpoint.scope !== expectedScope.scope || signed.checkpoint.artifactKind !== expectedScope.artifactKind) throw new IntegrityError('checkpoint scope mismatch');
  const signature = Uint8Array.from(signed.signature.match(/../g)!.map(x=>parseInt(x,16)));
  if (!await verifyAsync(signature,bytes,checkpointPublicKey(trustedOwner))) throw new IntegrityError('invalid checkpoint signature');
  return JSON.parse(JSON.stringify(signed.checkpoint)) as RecoveryCheckpoint;
}
/** Authenticate before supplying trusted authors/heads to A2A recovery. */
export async function checkpointA2ARestoreOptions(signed: SignedRecoveryCheckpoint, trustedOwner: string, context: string): Promise<A2ARestoreOptions> {
  const checkpoint = await verifyRecoveryCheckpoint(signed,trustedOwner,{artifactKind:'a2a',scope:context});
  const parentLocators: NonNullable<A2ARestoreOptions['parentLocators']> = Object.create(null);
  for (const entry of checkpoint.locators) {
    if (Object.hasOwn(parentLocators,entry.artifactId)) throw new UserError('A2A checkpoint adapter requires one selected locator per artifact');
    parentLocators[entry.artifactId] = {author:entry.author,locator:entry.locator};
  }
  return {expectedAuthors:checkpoint.expectedAuthors,heads:checkpoint.heads,parentLocators};
}
