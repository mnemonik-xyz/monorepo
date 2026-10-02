// Caller-owned ciphertext survives CLI process exit and failed delivery.
import { mkdirSync, readFileSync, writeFileSync, renameSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { configDir, restrictFileMode } from "./config.js";
import { UserError } from "./errors.js";

function pathFor(author: string, hash: string): string {
  if (!/^[1-9A-HJ-NP-Za-km-z]+$/.test(author) || !/^[a-f0-9]{64}$/.test(hash)) {
    throw new UserError("invalid local sealed memory identifier");
  }
  return join(configDir(), "sealed", author, `${hash}.json`);
}

export function saveSealed(author: string, row: { memoryHash: string; outerCbor: Uint8Array; signedBytes: Uint8Array }): string {
  const path = pathFor(author, row.memoryHash);
  mkdirSync(join(configDir(), "sealed", author), {recursive:true, mode:0o700});
  const tmp = `${path}.${randomUUID()}.tmp`;
  try {
    writeFileSync(tmp, JSON.stringify({version:1, author, memoryHash:row.memoryHash, outerCbor:Buffer.from(row.outerCbor).toString("base64"), signedBytes:Buffer.from(row.signedBytes).toString("base64")}), {mode:0o600, flag:"wx"});
    restrictFileMode(tmp);
    renameSync(tmp, path);
  } finally { rmSync(tmp, {force:true}); }
  return path;
}

export function loadSealed(author: string, hash: string): {outerCbor:Uint8Array; signedBytes:Uint8Array} {
  const path = pathFor(author, hash);
  try {
    if (statSync(path).size > 3 * 1024 * 1024) throw new Error("oversized record");
    const row = JSON.parse(readFileSync(path,"utf8"));
    if (row.version !== 1 || row.author !== author || row.memoryHash !== hash || typeof row.outerCbor !== "string" || typeof row.signedBytes !== "string") throw new Error("invalid record");
    const outerCbor = new Uint8Array(Buffer.from(row.outerCbor,"base64"));
    const signedBytes = new Uint8Array(Buffer.from(row.signedBytes,"base64"));
    if (!outerCbor.length || !signedBytes.length) throw new Error("empty record");
    return {outerCbor,signedBytes};
  } catch (e) {
    throw new UserError(`local sealed memory unavailable: ${path}. Restore your local ciphertext backup. Hosted hash lookup is retired; existing share links use the legacy reader.`, e);
  }
}
