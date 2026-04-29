// `mnemonic login` — headless save + interactive state-mismatch (TDD anchor).
//
// Driving the loopback server: we wrap `node:http`'s `createServer` via a
// `vi.mock` factory so we can capture the bound port and POST a wrong-state
// callback to it. Spying directly on the frozen ESM export does not work
// (TypeError: Cannot redefine property), hence the factory-level mock.

import { existsSync } from "node:fs";
import { join } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { runLogin } from "../src/commands/login.js";
import { AuthError } from "../src/errors.js";
import {
  clearWasmMock,
  installWasmMock,
  makeJwt,
  withTmpConfigDir,
} from "./helpers.js";

describe("runLogin (headless --token)", () => {
  let cleanup = (): void => {};
  let dir = "";

  beforeEach(() => {
    const r = withTmpConfigDir();
    dir = r.dir;
    cleanup = r.cleanup;
    installWasmMock();
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    cleanup();
  });

  it("persists a valid HS256 JWT", async () => {
    const jwt = makeJwt("user-pubkey-1");
    await runLogin({ token: jwt });
    expect(existsSync(join(dir, "token.json"))).toBe(true);
  });

  it("rejects an expired JWT (AuthError, exit 4)", async () => {
    const jwt = makeJwt("user-pubkey-1", -10);
    await expect(runLogin({ token: jwt })).rejects.toMatchObject({
      exitCode: 4,
    });
  });

  it("rejects an alg=none JWT", async () => {
    const b64 = (o: unknown) =>
      Buffer.from(JSON.stringify(o), "utf8")
        .toString("base64")
        .replace(/=+$/g, "")
        .replace(/\+/g, "-")
        .replace(/\//g, "_");
    const header = b64({ alg: "none", typ: "JWT" });
    const payload = b64({
      sub: "x",
      iat: Math.floor(Date.now() / 1000),
      exp: Math.floor(Date.now() / 1000) + 3600,
    });
    const bad = `${header}.${payload}.`;
    await expect(runLogin({ token: bad })).rejects.toMatchObject({
      exitCode: 4,
    });
  });
});

// TDD anchor: state-mismatch on the loopback callback aborts login,
// no token written, exit 4.
//
// We drive the callback by patching the prototype-level `Server.listen` to
// emit a `listening` event we can hook before/after binding (the Server
// emits this event natively, so we just attach an extra listener via a
// side-channel: we hook server.address() after listen by polling via an
// interval).
describe("runLogin interactive state-mismatch (TDD anchor)", () => {
  let cleanup = (): void => {};
  let dir = "";

  beforeEach(() => {
    const r = withTmpConfigDir();
    dir = r.dir;
    cleanup = r.cleanup;
    installWasmMock();
    vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    vi.spyOn(process.stderr, "write").mockImplementation(() => true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    clearWasmMock();
    cleanup();
  });

  it("aborts with AuthError when callback state ≠ stored state", async () => {
    // Patch Server.prototype.listen so every newly-bound server gets a
    // `listening` listener that fires the wrong-state callback at it.
    const httpMod = await import("node:http");
    let driverFired = false;
    const proto = httpMod.Server.prototype as unknown as {
      listen: (...args: unknown[]) => unknown;
    };
    const origListen = proto.listen;
    proto.listen = function patched(
      this: InstanceType<typeof httpMod.Server>,
      ...args: unknown[]
    ) {
      // Once this server starts listening, drive the wrong-state callback.
      this.once("listening", () => {
        if (driverFired) return;
        driverFired = true;
        const addr = this.address();
        if (addr && typeof addr === "object" && "port" in addr) {
          const port = (addr as { port: number }).port;
          setImmediate(() => {
            fetch(
              `http://127.0.0.1:${port}/callback?code=ABC&state=DEFINITELY-WRONG-STATE`
            ).catch(() => {
              /* swallow */
            });
          });
        }
      });
      return origListen.apply(this, args as never);
    };

    try {
      await expect(
        runLogin({
          baseUrl: "http://idp.test",
          noOpen: true,
          timeoutMs: 10_000,
        })
      ).rejects.toBeInstanceOf(AuthError);
    } finally {
      proto.listen = origListen;
    }

    expect(existsSync(join(dir, "token.json"))).toBe(false);
  }, 15_000);
});
