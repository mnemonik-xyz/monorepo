// Sealed writes persist signed ciphertext locally before optional external delivery.
// --public --anchor uses the legacy plaintext preparation flow.
//
// Content sources:
//   1. positional argument (preferred)
//   2. stdin if argument is missing AND stdin is not a TTY
//   3. otherwise: UserError "no content provided"
//
// Tags from `--tags=a,b,c` (comma-separated, trimmed, empty entries dropped).

import { saveSealed } from "../sealed-store.js";

import { AuthError, parseJwtPayload } from "@mnemonik-xyz/sdk";

import { identityPath, identitySecretInFile, tokenPath } from "../config.js";
import { fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions, verbose } from "../output.js";
import { formatMismatchError } from "../preflight.js";
import { openSession, openLocalSession } from "../session.js";

export interface SignOptions extends OutputOptions {
  tags?: string;
  baseUrl?: string;
  /** `--anchor` (alias `--participate`): seal + anchor on-chain (reads keychain). */
  anchor?: boolean;
  /** `--public`: plaintext write (legacy signMemory path). */
  public?: boolean;
  /** Internal — read content from this string instead of stdin (tests). */
  content?: string;
}

const DEFAULT_BASE_URL = "https://mcp.mnemonik.xyz";

export async function runSign(
  positional: string | undefined,
  opts: SignOptions
): Promise<void> {
  const baseUrl =
    opts.baseUrl ?? process.env.MNEMONIC_BASE_URL ?? DEFAULT_BASE_URL;

  const content = await resolveContent(positional, opts);
  if (!content) {
    throw new UserError(
      "no content provided; pass as a positional argument or pipe via stdin"
    );
  }

  const tags = parseTags(opts.tags);
  const anchor = opts.anchor === true;
  const isPublic = opts.public === true;

  if (isPublic && !anchor) throw new UserError("--public without --anchor is no longer supported over HTTP. Use default sealed local storage, or explicitly choose --public --anchor for permanent public disclosure.");

  if (isPublic) {
    // --public: plaintext signMemory path (unchanged legacy behavior).
    await runSignPublic(content, tags, anchor, baseUrl, opts);
  } else {
    // Default or --anchor: sealed sealMemory path.
    await runSignSealed(content, tags, anchor, baseUrl, opts);
  }
}

// ---------------------------------------------------------------------------
// Sealed path (default / --anchor)
// ---------------------------------------------------------------------------

async function runSignSealed(
  content: string,
  tags: string[],
  anchor: boolean,
  baseUrl: string,
  opts: SignOptions
): Promise<void> {
  // Sealed write needs the keypair for E2E encryption.  We never read the OS
  // keychain unless the command also reads it anyway (--anchor) OR the key is
  // file-backed (no prompt possible).
  const { client, signer } = anchor
    ? await openSession(baseUrl, opts, true)
    : openLocalSession(baseUrl);

  verbose(`base_url=${baseUrl}`, opts);
  verbose(`local pubkey=${signer.pubkey}`, opts);
  verbose(`mode=${anchor ? "seal+anchor" : "seal+store"}`, opts);

  // Provide the keypair lazily — refuse keychain reads unless --anchor or
  // the secret is already in a file.
  client.setKeypairProvider(() => {
    if (anchor || identitySecretInFile()) return signer.keypair();
    throw new UserError(
      "a sealed local write needs your encryption key, but it is stored in " +
        "the OS keychain and this command does not read the keychain by default. " +
        "Options:\n" +
        "  • `mnemonic sign --anchor` to seal and anchor (reads the keychain once)\n" +
        "  • restore a file-backed identity to seal locally without a keychain prompt",
    );
  });

  hint(anchor ? "sealing and anchoring memory..." : "sealing memory...", opts);
  let result;
  try {
    result = await client.sealMemory(content, {
      mode: "store",
      ...(tags.length > 0 ? { tags } : {}),
    });
    if (!result.outerCbor || !result.signedBytes) throw new UserError("SDK did not return original sealed bytes; upgrade to a client-prepared-memory SDK before saving");
    const localPath = saveSealed(signer.pubkey, {memoryHash:result.memoryHash,outerCbor:result.outerCbor,signedBytes:result.signedBytes});
    if (anchor) {
      try {
        const receipt = await client.ingestPreparedMemory(result.signedBytes);
        if (receipt.content_hash !== result.memoryHash || receipt.author !== signer.pubkey) throw new UserError("delivery receipt does not match the prepared memory");
        result = {...result, locator: receipt.locator as string, receipt};
      } catch (e) {
        hint(`Original signed ciphertext retained at ${localPath}. Retry these bytes with SDK ingestPreparedMemory; do not re-run sign to resume a paid operation.`, opts);
        throw e;
      }
    }
  } catch (e) {
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    return `memory_hash: ${result.memoryHash}`;
  });
}

// ---------------------------------------------------------------------------
// Public (plaintext) path  — --public flag
// ---------------------------------------------------------------------------

async function runSignPublic(
  content: string,
  tags: string[],
  anchor: boolean,
  baseUrl: string,
  opts: SignOptions
): Promise<void> {
  const mode = "anchored";
  const { client, signer, token: tok } = await openSession(baseUrl, opts, true);
  verbose(`base_url=${baseUrl}`, opts);
  verbose(`local pubkey=${signer.pubkey}`, opts);
  verbose(`mode=${mode} (public/plaintext legacy preparation)`, opts);
  client.setKeypairProvider(() => signer.keypair());

  hint(anchor ? "signing and anchoring memory..." : "saving memory...", opts);
  let result;
  try {
    result = await client.signMemory(content, {
      mode,
      ...(tags.length > 0 ? { tags } : {}),
    });
  } catch (e) {
    // Post-mortem on 403 from /api/sign-callback: the only way that can
    // happen is `pending.jwt_sub !== body.signer_pubkey`. Preflight already
    // checks the saved fields, but a stale file-read, a token-rotation race,
    // or a server quirk can still bypass it. Re-derive the JWT's actual
    // payload from the wire-side token and surface the discrepancy with the
    // same remediation hints preflight uses, so the user sees a directly
    // actionable error instead of `HTTP 403`.
    if (e instanceof AuthError && /HTTP 403/.test(e.message)) {
      throw new UserError(buildPostMortem(signer.pubkey, tok.jwt), e);
    }
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    const lines = [
      `attestation_id: ${result.attestationId}`,
      `signed_at:      ${result.signedAt}`,
      `status:         ${result.status}`,
    ];
    if (result.writeMode) lines.push(`write_mode:     ${result.writeMode}`);
    if (result.contentHash) lines.push(`content_hash:   ${result.contentHash}`);
    if (result.arweaveTx) lines.push(`arweave_tx:     ${result.arweaveTx}`);
    if (result.solanaTx) lines.push(`solana_tx:      ${result.solanaTx}`);
    return lines.join("\n");
  });
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

async function resolveContent(
  positional: string | undefined,
  opts: SignOptions
): Promise<string> {
  if (positional && positional.length > 0) return positional;
  if (typeof opts.content === "string") return opts.content;
  // stdin only if it is piped (not a TTY).
  if (process.stdin.isTTY) return "";
  return readStdin();
}

function readStdin(): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => {
      data += chunk;
    });
    process.stdin.on("end", () => resolve(data.replace(/\r?\n$/, "")));
    process.stdin.on("error", reject);
  });
}

function parseTags(raw: string | undefined): string[] {
  if (!raw) return [];
  return raw
    .split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}

/**
 * Build the post-mortem message shown when `/api/sign-callback` returned
 * 403. Re-decodes the JWT payload to learn the real `sub` (in case
 * `token.json.sub` had drifted from `jwt.sub`), then either surfaces the
 * mismatch using the standard preflight message OR falls back to a
 * server-side hint when the local view is internally consistent.
 */
function buildPostMortem(localPubkey: string, jwt: string): string {
  let jwtSub: string;
  try {
    jwtSub = parseJwtPayload(jwt).sub;
  } catch {
    jwtSub = "(unparseable JWT)";
  }
  if (localPubkey !== jwtSub) {
    return formatMismatchError({
      identityPubkey: localPubkey,
      tokenSub: jwtSub,
      identityPath: identityPath(),
      tokenPath: tokenPath(),
    });
  }
  return [
    "sign-callback rejected (HTTP 403) although local identity matches JWT.sub.",
    `  local pubkey:  ${localPubkey}`,
    `  JWT.sub:       ${jwtSub}`,
    "",
    "Most likely the server stored a different `jwt_sub` for the pending bundle",
    "than the JWT actually carries. Possible causes:",
    "  • the JWT was minted before a server-side identity rotation",
    "  • the token at token.json is from a different deployment",
    "",
    "Try: rerun `mnemonic login` to mint a fresh JWT, then `mnemonic sign` again.",
    "If the problem persists, run with `--verbose` and report the output.",
  ].join("\n");
}
