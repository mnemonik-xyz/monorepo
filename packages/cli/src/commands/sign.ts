// `mnemonic sign [content]` — read identity + token, instantiate a
// MnemonicClient, run the pending-bundle / sign-callback flow.
//
// Content sources:
//   1. positional argument (preferred)
//   2. stdin if argument is missing AND stdin is not a TTY
//   3. otherwise: UserError "no content provided"
//
// Tags from `--tags=a,b,c` (comma-separated, trimmed, empty entries dropped).

import {
  AuthError,
  LocalSigner,
  MnemonicClient,
  PaymentRequiredError,
  parseJwtPayload,
} from "@mnemonik-xyz/sdk";

import {
  identityPath,
  loadIdentity,
  loadToken,
  tokenPath,
} from "../config.js";
import { fromSdkError, UserError } from "../errors.js";
import { format, hint, type OutputOptions, verbose } from "../output.js";
import {
  assertIdentityMatchesToken,
  formatMismatchError,
} from "../preflight.js";

export interface SignOptions extends OutputOptions {
  tags?: string;
  baseUrl?: string;
  mode?: "local" | "participate";
  visibility?: "private" | "public";
  checkpointType?: "manual" | "pre_compaction" | "session_end";
  workspace?: string;
  openPayment?: boolean;
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
  // Pre-flight: catch identity/JWT mismatch BEFORE any fetch is built (bug 3 /
  // Decision 7). UserError points at three remediation paths.
  assertIdentityMatchesToken();
  const kp = await loadIdentity();
  const tok = loadToken();

  verbose(`base_url=${baseUrl}`, opts);
  verbose(`local pubkey=${kp.pubkey}`, opts);
  verbose(`token.sub=${tok.sub}`, opts);

  const client = new MnemonicClient({
    baseUrl,
    signer: new LocalSigner(kp),
    jwt: tok.jwt,
  });
  client.setKeypair(kp);

  hint("signing memory...", opts);
  let result;
  try {
    result = await client.signMemory(content, {
      ...(tags.length > 0 ? { tags } : {}),
      mode: opts.mode ?? "local",
      ...(opts.visibility ? { visibility: opts.visibility } : {}),
      ...(opts.checkpointType ? { checkpointType: opts.checkpointType } : {}),
      ...(opts.workspace ? { workspace: opts.workspace } : {}),
    });
  } catch (e) {
    if (e instanceof PaymentRequiredError) {
      const { session, exact } = paymentUrls(e, baseUrl);
      const pending = {
        status: "payment_required",
        operation_id: e.challenge.operationId,
        amount: e.challenge.quote.amount,
        asset: e.challenge.quote.asset,
        session_url: session,
        pay_once_url: exact,
        expires_at: e.challenge.quote.expires_at,
      };
      format(pending, opts, () =>
        [
          "This signed memory is waiting for anchoring payment.",
          `Start a capped seamless session: ${session}`,
          `Optional pay-once x402:        ${exact}`,
          "The signed operation is stored and can be resumed without signing or paying twice.",
        ].join("\n"),
      );
      if (
        opts.openPayment !== false &&
        !opts.json &&
        !opts.quiet &&
        process.stdout.isTTY
      ) {
        try {
          const { default: open } = await import("open");
          await open(session);
        } catch {
          // The printed URL is the reliable fallback on headless systems.
        }
      }
      return;
    }
    // Post-mortem on 403 from /api/sign-callback: the only way that can
    // happen is `pending.jwt_sub !== body.signer_pubkey`. Preflight already
    // checks the saved fields, but a stale file-read, a token-rotation race,
    // or a server quirk can still bypass it. Re-derive the JWT's actual
    // payload from the wire-side token and surface the discrepancy with the
    // same remediation hints preflight uses, so the user sees a directly
    // actionable error instead of `HTTP 403`.
    if (e instanceof AuthError && /HTTP 403/.test(e.message)) {
      throw new UserError(buildPostMortem(kp.pubkey, tok.jwt), e);
    }
    throw fromSdkError(e);
  }

  format(result, opts, (_d, _color) => {
    const lines = [
      `attestation_id: ${result.attestationId}`,
      `signed_at:      ${result.signedAt}`,
      `status:         ${result.status}`,
    ];
    if (result.contentHash) lines.push(`content_hash:   ${result.contentHash}`);
    if (result.arweaveTx) lines.push(`arweave_tx:     ${result.arweaveTx}`);
    if (result.solanaTx) lines.push(`solana_tx:      ${result.solanaTx}`);
    return lines.join("\n");
  });
}

function paymentUrls(
  error: PaymentRequiredError,
  baseUrl: string,
): { session: string; exact: string } {
  const stake = error.challenge.quote.accepts.find(
    (option) => option.scheme === "stake",
  );
  const raw =
    stake && typeof stake.payment_url === "string"
      ? stake.payment_url
      : "https://mnemonik-dev.github.io/universal-paywall-site/";
  const build = (scheme: "stake" | "exact") => {
    const url = new URL(raw);
    url.searchParams.set("operation_id", error.challenge.operationId);
    url.searchParams.set("scheme", scheme);
    url.searchParams.set("mcp_base", baseUrl);
    return url.toString();
  };
  return { session: build("stake"), exact: build("exact") };
}

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
