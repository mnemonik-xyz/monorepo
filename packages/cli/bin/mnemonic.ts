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
} from "../src/commands/identity.js";
import { handleError } from "../src/errors.js";
import type { OutputOptions } from "../src/output.js";

interface RootFlags {
  json?: boolean;
  quiet?: boolean;
  color?: boolean; // commander negates --no-color into `color: false`
}

function rootOpts(program: Command): OutputOptions {
  const flags = program.opts<RootFlags>();
  return {
    json: Boolean(flags.json),
    quiet: Boolean(flags.quiet),
    noColor: flags.color === false,
  };
}

export function buildProgram(): Command {
  const program = new Command();
  program
    .name("mnemonic")
    .description("Mnemonic Protocol CLI — verifiable persistent memory")
    .version("0.0.0")
    .option("--json", "machine-readable JSON output")
    .option("--quiet", "suppress non-essential output")
    .option("--no-color", "disable ANSI color");

  program
    .command("init")
    .description("generate a fresh keypair at ~/.mnemonic/identity.json")
    .option("--force", "overwrite existing identity")
    .action(async (cmdOpts: { force?: boolean }) => {
      await runInit({
        ...rootOpts(program),
        ...(cmdOpts.force !== undefined ? { force: cmdOpts.force } : {}),
      });
    });

  program
    .command("login")
    .description("OAuth login (interactive PKCE loopback) or --token <jwt>")
    .option("--token <jwt>", "headless: persist a pre-issued JWT")
    .option("--base-url <url>", "override the server base URL")
    .action(async (cmdOpts: { token?: string; baseUrl?: string }) => {
      await runLogin({
        ...rootOpts(program),
        ...(cmdOpts.token !== undefined ? { token: cmdOpts.token } : {}),
        ...(cmdOpts.baseUrl !== undefined ? { baseUrl: cmdOpts.baseUrl } : {}),
      });
    });

  program
    .command("sign [content]")
    .description("sign a memory (content from arg or stdin)")
    .option("--tags <list>", "comma-separated tags")
    .option("--base-url <url>", "override the server base URL")
    .action(
      async (
        content: string | undefined,
        cmdOpts: { tags?: string; baseUrl?: string }
      ) => {
        await runSign(content, {
          ...rootOpts(program),
          ...(cmdOpts.tags !== undefined ? { tags: cmdOpts.tags } : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      }
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
        cmdOpts: { topK?: number; tag?: string; baseUrl?: string }
      ) => {
        await runRecall(query, {
          ...rootOpts(program),
          ...(cmdOpts.topK !== undefined ? { topK: cmdOpts.topK } : {}),
          ...(cmdOpts.tag !== undefined ? { tag: cmdOpts.tag } : {}),
          ...(cmdOpts.baseUrl !== undefined
            ? { baseUrl: cmdOpts.baseUrl }
            : {}),
        });
      }
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
      }
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

  return program;
}

export async function main(argv: string[]): Promise<void> {
  const program = buildProgram();
  try {
    await program.parseAsync(argv);
  } catch (e) {
    handleError(e);
  }
}

// Only run when invoked directly (so test files can `import { buildProgram }`).
const invokedDirectly =
  typeof process !== "undefined" &&
  Array.isArray(process.argv) &&
  process.argv[1] !== undefined &&
  (process.argv[1].endsWith("/mnemonic.ts") ||
    process.argv[1].endsWith("/mnemonic.js") ||
    process.argv[1].endsWith("\\mnemonic.ts") ||
    process.argv[1].endsWith("\\mnemonic.js") ||
    process.argv[1].endsWith("/bin/mnemonic"));

if (invokedDirectly) {
  void main(process.argv);
}
