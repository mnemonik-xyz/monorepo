// `bin/mnemonic.ts` smoke — `--help` lists every command and identity has
// import + export subcommands. Drives Decision 10 + the spec-required
// surface listed in tasks/5.md.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { buildProgram } from "../bin/mnemonic.js";

describe("buildProgram", () => {
  beforeEach(() => {
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("registers all 8 top-level commands", () => {
    const p = buildProgram();
    const names = p.commands.map((c) => c.name());
    expect(names).toEqual(
      expect.arrayContaining([
        "init",
        "login",
        "sign",
        "recall",
        "verify",
        "whoami",
        "prove",
        "identity",
      ])
    );
  });

  it("identity has both import and export subcommands", () => {
    const p = buildProgram();
    const identity = p.commands.find((c) => c.name() === "identity");
    expect(identity).toBeDefined();
    const sub = (
      identity as { commands: Array<{ name(): string }> }
    ).commands.map((c) => c.name());
    expect(sub).toEqual(expect.arrayContaining(["import", "export"]));
  });

  it("--version prints the package version", () => {
    let captured = "";
    vi.spyOn(process.stdout, "write").mockImplementation((c: unknown) => {
      captured += String(c);
      return true;
    });
    const p = buildProgram();
    p.exitOverride(); // throw instead of process.exit on --version
    expect(() => p.parse(["node", "mnemonic", "--version"])).toThrow();
    expect(captured).toContain("0.1.4");
  });

  it("recognises top-level flags --json / --quiet / --no-color", () => {
    const p = buildProgram();
    p.exitOverride();
    // Use a fake action — `init --force` would actually run; we only want
    // to exercise option parsing. Inject a no-op action on init.
    const init = p.commands.find((c) => c.name() === "init");
    expect(init).toBeDefined();
    init?.action(() => {
      /* no-op */
    });
    p.parse(["node", "mnemonic", "--json", "--quiet", "--no-color", "init"]);
    const opts = p.opts<{ json?: boolean; quiet?: boolean; color?: boolean }>();
    expect(opts.json).toBe(true);
    expect(opts.quiet).toBe(true);
    expect(opts.color).toBe(false);
  });
});
