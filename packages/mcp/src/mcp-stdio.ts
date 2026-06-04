import { spawn } from "node:child_process";

import { ensureBinaryCached } from "./install-binary.js";

/**
 * Spawn the cached binary's `mcp-stdio` subcommand and forward stdio.
 * Resolves with the child's exit code (or 1 on signal); the bin entrypoint
 * propagates this to `process.exit`.
 */
export async function mcpStdio(extraArgs: string[] = []): Promise<number> {
  const bin = await ensureBinaryCached();
  return new Promise<number>((resolveP, rejectP) => {
    const child = spawn(bin, ["mcp-stdio", ...extraArgs], {
      stdio: ["inherit", "inherit", "inherit"],
    });
    child.on("error", rejectP);
    child.on("close", (code, signal) => {
      if (code !== null) {
        resolveP(code);
        return;
      }
      // Subprocess killed by signal — propagate as non-zero so the host
      // doesn't treat the exit as success.
      process.stderr.write(
        `mnemonik-mcp: cached binary terminated by signal ${signal ?? "unknown"}\n`,
      );
      resolveP(1);
    });
  });
}
