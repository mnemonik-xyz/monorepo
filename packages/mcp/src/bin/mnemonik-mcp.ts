#!/usr/bin/env node
// @mnemonik-xyz/mcp — `mnemonik-mcp` binary entrypoint.
//
// No `postinstall` (security hardening, Decision 8). First invocation
// lazy-installs the platform-matching Rust binary via `ensureBinaryCached`.

import { doctor } from "../doctor.js";
import { installHosts } from "../install-hosts.js";
import { mcpStdio } from "../mcp-stdio.js";

const USAGE = `\
mnemonik-mcp — MCP shim for Mnemonic Protocol

Usage:
  mnemonik-mcp install [--confirm]    Wire mcpServers.mnemonik into present host configs
  mnemonik-mcp install --check         Dry-run install (no writes)
  mnemonik-mcp mcp-stdio               Spawn cached binary, forward MCP JSON-RPC on stdio
  mnemonik-mcp doctor                  Run six diagnostic checks
`;

export async function main(argv: string[]): Promise<number> {
  const args = argv.slice(2);
  const sub = args[0];

  if (sub === undefined || sub === "--help" || sub === "-h" || sub === "help") {
    process.stdout.write(USAGE);
    // No subcommand = exit 1 per spec; explicit --help = exit 0.
    return sub === undefined ? 1 : 0;
  }

  switch (sub) {
    case "install": {
      const check = args.includes("--check");
      const confirm = args.includes("--confirm");
      await installHosts({ check, confirm });
      return 0;
    }
    case "mcp-stdio": {
      return mcpStdio(args.slice(1));
    }
    case "doctor": {
      const result = await doctor();
      return result.exitCode;
    }
    default: {
      process.stderr.write(`unknown subcommand: ${sub}\n\n${USAGE}`);
      return 1;
    }
  }
}

// Run when invoked directly. Mirrors @mnemonik-xyz/cli's invokedDirectly check
// so test files can `import { main }` without spawning the CLI.
const argv1 = process.argv[1] ?? "";
const invokedDirectly =
  typeof process !== "undefined" &&
  Array.isArray(process.argv) &&
  argv1.length > 0 &&
  (argv1.endsWith("/mnemonik-mcp.ts") ||
    argv1.endsWith("/mnemonik-mcp.js") ||
    argv1.endsWith("\\mnemonik-mcp.ts") ||
    argv1.endsWith("\\mnemonik-mcp.js") ||
    /[\\/]mnemonik-mcp$/.test(argv1));

if (invokedDirectly) {
  try {
    const code = await main(process.argv);
    process.exit(code);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    process.stderr.write(`mnemonik-mcp: ${msg}\n`);
    process.exit(1);
  }
}
