#!/usr/bin/env node
// @mnemonik-xyz/cli — `mnemonic` binary entrypoint.
//
// Argv routing via `commander`. Top-level flags `--json`, `--quiet`,
// `--no-color` flow through `OutputOptions` to every command. All exits go
// through `handleError` which maps typed CLI errors to documented exit codes
// (Decision 10): 0=ok, 1=user, 2=server, 3=integrity, 4=auth.

import { Command } from "commander";

import { runInit } from "../src/commands/init.js";
import { runLogin } from "../src/commands/login.js";
import { runSign } from "../src/commands/sign.js";
import { runRecall } from "../src/commands/recall.js";
import { runVerify } from "../src/commands/verify.js";
import { runWhoami } from "../src/commands/whoami.js";
import { runProve } from "../src/commands/prove.js";
import {
  runIdentityImport,
  runIdentityExport,
  statusCommand,
  pullFromWebappCommand,
  pushToWebappCommand,
} from "../src/commands/identity.js";
import { handleError } from "../src/errors.js";
import { ensure, shouldSkipEnsure } from "../src/identity/ensure.js";
import type { OutputOptions } from "../src/output.js";

// Re-export so callers that previously imported shouldSkipEnsure from this
// module don't break.
export { shouldSkipEnsure };

interface RootFlags {
  json?: boolean;
  quiet?: boolean;
  color?: boolean; // commander negates --no-color into `color: false`
  verbose?: boolean;
}

function rootOpts(program: Command): OutputOptions {
  const flags = program.opts<RootFlags>();
  return {
    json: Boolean(flags.json),
    quiet: Boolean(flags.quiet),
    noColor: flags.color === false,
    verbose: Boolean(flags.verbose),
  };
}

export function buildProgram(): Command {
  const program = new Command();
  program
    .name("mnemonic")
    .description("Mnemonic Protocol CLI — verifiable persistent memory")
    // TODO: read from package.json at build/runtime so we don't have
    // to hand-bump on every release. Hardcoded for 0.1.7 — the
    // browserless-OAuth fix (#27); 0.1.6 was tagged but never published
    // to npm, so we skip to 0.1.7 to keep tag history monotonic.
    .version("0.1.7")
    .option("--json", "machine-readable JSON output")
    .option("--quiet", "suppress non-essential output")
    .option("--no-color", "disable ANSI color")
    .option(
      "--verbose",
      "log auth identifiers + HTTP request/response context to stderr",
    );

  program
    .command("init")
    .description(
      "set up CLI identity — pair with webapp via --ticket (recommended) or --standalone",
    )
    .option(
      "--ticket <uuid>",
      "redeem a webapp 'Send to CLI' ticket (recommended — keeps CLI + webapp keypairs aligned)",
    )
    .option(
      "--standalone",
      "generate a fresh CLI-only keypair (advanced; will not match webapp localStorage)",
    )
    .option("--force", "overwrite existing identity")
    .option(
      "--base-url <url>",
      "override the server base URL (used with --ticket)",
    )
    .action(
      async (cmdOpts: {
        ticket?: string;
        standalone?: boolean;
        force?: boolean;
        baseUrl?: string;
      }) => {
        await runInit({
          ...rootOpts(program),
          ...(cmdOpts.ticket !== undefined ? { ticket: cmdOpts.ticket } : {}),
          ...(cmdOpts.standalone !== undefined
            ? { standalone: cmdOpts.standalone }
            : {}),
          ...(cmdOpts.force !== undefined ? { force: cmdOpts.force } : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      },
    );

  program
    .command("login")
    .description(
      "OAuth login — default signs the server challenge with the local CLI keypair (browserless). Use --browser for the legacy webapp-localStorage flow, or --token <jwt> for a pre-issued JWT.",
    )
    .option("--token <jwt>", "headless: persist a pre-issued JWT")
    .option(
      "--browser",
      "use the legacy browser-mediated OAuth flow (webapp localStorage signs the challenge)",
    )
    .option("--base-url <url>", "override the server base URL")
    .action(
      async (cmdOpts: {
        token?: string;
        browser?: boolean;
        baseUrl?: string;
      }) => {
        await runLogin({
          ...rootOpts(program),
          ...(cmdOpts.token !== undefined ? { token: cmdOpts.token } : {}),
          ...(cmdOpts.browser !== undefined
            ? { browser: cmdOpts.browser }
            : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      },
    );

  program
    .command("sign [content]")
    .description("sign a memory (content from arg or stdin)")
    .option("--tags <list>", "comma-separated tags")
    .option("--base-url <url>", "override the server base URL")
    .action(
      async (
        content: string | undefined,
        cmdOpts: { tags?: string; baseUrl?: string },
      ) => {
        await runSign(content, {
          ...rootOpts(program),
          ...(cmdOpts.tags !== undefined ? { tags: cmdOpts.tags } : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      },
    );

  program
    .command("recall <query>")
    .description("recall similar memories")
    .option("--top-k <n>", "max hits to return", (v) => parseInt(v, 10), 5)
    .option("--tag <tag>", "filter by a single tag")
    .option("--base-url <url>", "override the server base URL")
    .action(
      async (
        query: string,
        cmdOpts: { topK?: number; tag?: string; baseUrl?: string },
      ) => {
        await runRecall(query, {
          ...rootOpts(program),
          ...(cmdOpts.topK !== undefined ? { topK: cmdOpts.topK } : {}),
          ...(cmdOpts.tag !== undefined ? { tag: cmdOpts.tag } : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      },
    );

  program
    .command("verify <attestation_id>")
    .description("verify an attestation (exit: 0 ok, 3 tampered, 1 not found)")
    .option("--base-url <url>", "override the server base URL")
    .action(async (id: string, cmdOpts: { baseUrl?: string }) => {
      await runVerify(id, {
        ...rootOpts(program),
        ...(cmdOpts.baseUrl !== undefined ? { baseUrl: cmdOpts.baseUrl } : {}),
      });
    });

  program
    .command("whoami")
    .description("show your local identity + JWT (client-side, no server call)")
    .option("--with-count", "also fetch total memory count from server")
    .option("--base-url <url>", "override the server base URL")
    .action(async (cmdOpts: { withCount?: boolean; baseUrl?: string }) => {
      await runWhoami({
        ...rootOpts(program),
        ...(cmdOpts.withCount !== undefined
          ? { withCount: cmdOpts.withCount }
          : {}),
        ...(cmdOpts.baseUrl !== undefined ? { baseUrl: cmdOpts.baseUrl } : {}),
      });
    });

  program
    .command("prove")
    .description("sign a challenge with your local key (offline)")
    .option("--challenge <hex>", "hex challenge bytes (default: 32 random)")
    .action(async (cmdOpts: { challenge?: string }) => {
      await runProve({
        ...rootOpts(program),
        ...(cmdOpts.challenge !== undefined
          ? { challenge: cmdOpts.challenge }
          : {}),
      });
    });

  // identity {import, export}
  const identity = program
    .command("identity")
    .description("manage the keypair file (import via ticket, export to disk)");

  identity
    .command("import")
    .description("import a keypair from a webapp ticket or a JSON file")
    .option("--ticket <uuid>", "redeem a webapp 'Send to CLI' ticket")
    .option("--file <path>", "read JSON keypair from a file")
    .option("--force", "overwrite existing identity")
    .option("--base-url <url>", "override the server base URL")
    .action(
      async (cmdOpts: {
        ticket?: string;
        file?: string;
        force?: boolean;
        baseUrl?: string;
      }) => {
        await runIdentityImport({
          ...rootOpts(program),
          ...(cmdOpts.ticket !== undefined ? { ticket: cmdOpts.ticket } : {}),
          ...(cmdOpts.file !== undefined ? { file: cmdOpts.file } : {}),
          ...(cmdOpts.force !== undefined ? { force: cmdOpts.force } : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      },
    );

  identity
    .command("export")
    .description("export the current identity to a file (mode 0600)")
    .option("--file <path>", "destination path (required)")
    .action(async (cmdOpts: { file?: string }) => {
      await runIdentityExport({
        ...rootOpts(program),
        ...(cmdOpts.file !== undefined ? { file: cmdOpts.file } : {}),
      });
    });

  identity
    .command("status")
    .description(
      "compare local identity (KeyStore/file) vs cached JWT — local-only, no network",
    )
    .action(async () => {
      const opts = rootOpts(program);
      const code = await statusCommand({
        ...(opts.json !== undefined ? { json: opts.json } : {}),
        ...(opts.noColor !== undefined ? { noColor: opts.noColor } : {}),
      });
      process.exit(code);
    });

  identity
    .command("pull-from-webapp [short-code]")
    .description(
      "adopt a CLI identity issued by the webapp — redeem the short code from `push-to-webapp`",
    )
    .option(
      "--stdin",
      "read short code from stdin instead of argv (avoids shell history leak)",
    )
    .option("--server-url <url>", "override the server base URL")
    .action(
      async (
        shortCodeArg: string | undefined,
        cmdOpts: { stdin?: boolean; serverUrl?: string },
      ) => {
        const code = await pullFromWebappCommand(shortCodeArg ?? "-", {
          stdin: cmdOpts.stdin ?? !shortCodeArg,
        });
        process.exit(code);
      },
    );

  identity
    .command("push-to-webapp")
    .description(
      "issue a ticket from the local CLI identity — prints a short code and QR for the webapp",
    )
    .option("--code-only", "print short code and URL only, skip the QR")
    .option("--qr-only", "print only the QR (no text output)")
    .option("--server-url <url>", "override the server base URL")
    .action(
      async (cmdOpts: {
        codeOnly?: boolean;
        qrOnly?: boolean;
        serverUrl?: string;
      }) => {
        const code = await pushToWebappCommand({
          ...(cmdOpts.codeOnly !== undefined
            ? { codeOnly: cmdOpts.codeOnly }
            : {}),
          ...(cmdOpts.qrOnly !== undefined ? { qrOnly: cmdOpts.qrOnly } : {}),
          ...(cmdOpts.serverUrl !== undefined
            ? { serverUrl: cmdOpts.serverUrl }
            : {}),
        });
        process.exit(code);
      },
    );

  return program;
}

export async function main(argv: string[]): Promise<void> {
  // Bootstrap identity before any command runs.  Skipped for help/version
  // flags and specific subcommands that manage identity themselves.
  if (!shouldSkipEnsure(argv)) {
    try {
      await ensure();
    } catch (e) {
      // Print a clean error without leaking secret bytes, then exit.
      const msg = e instanceof Error ? e.message : String(e);
      process.stderr.write(`mnemonic: identity bootstrap failed: ${msg}\n`);
      process.exit(1);
    }
  }

  const program = buildProgram();
  try {
    await program.parseAsync(argv);
  } catch (e) {
    handleError(e);
  }
}

// Only run when invoked directly (so test files can `import { buildProgram }`).
//
// Invocation paths we must recognize:
//   - dev/source: `bun run bin/mnemonic.ts` -> argv[1] ends with `/mnemonic.ts`
//   - compiled local: `node dist/bin/mnemonic.js` -> ends with `/mnemonic.js`
//   - npm-installed: `node_modules/.bin/mnemonic` (a symlink npm creates from
//     the `bin` field) -> argv[1] ends with `/.bin/mnemonic` or with no
//     extension at all on Windows shim variants.
//   - global install: `/usr/local/bin/mnemonic` -> ends with `/bin/mnemonic`
const argv1 = process.argv[1] ?? "";
const invokedDirectly =
  typeof process !== "undefined" &&
  Array.isArray(process.argv) &&
  argv1.length > 0 &&
  (argv1.endsWith("/mnemonic.ts") ||
    argv1.endsWith("/mnemonic.js") ||
    argv1.endsWith("\\mnemonic.ts") ||
    argv1.endsWith("\\mnemonic.js") ||
    // Matches `/usr/local/bin/mnemonic`, `node_modules/.bin/mnemonic`, and
    // any other directory whose basename is exactly `mnemonic`.
    /[\\/]mnemonic$/.test(argv1));

if (invokedDirectly) {
  // Top-level await — without this, Bun's runtime can exit before the
  // async chain resolves under certain timing conditions (the parent
  // script ends, no I/O is registered, the event loop drains the
  // microtask queue and exits). Surfaced by Task 8's CLI integration
  // tests (subprocess flakiness when spawning many bun children).
  await main(process.argv);
}
