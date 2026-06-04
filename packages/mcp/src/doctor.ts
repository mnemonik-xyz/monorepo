import { createHash } from "node:crypto";
import { mkdir, readFile, stat, unlink } from "node:fs/promises";
import { join } from "node:path";

import { hostCandidates } from "./install-hosts.js";
import { binaryPath, homedir, manifestPath } from "./paths.js";

interface CheckResult {
  name: string;
  pass: boolean;
  detail: string;
}

const HEALTH_URL = "https://mcp.mnemonik.xyz/health";

async function fileExists(p: string): Promise<boolean> {
  try {
    await stat(p);
    return true;
  } catch {
    return false;
  }
}

async function sha256OfFile(path: string): Promise<string> {
  const buf = await readFile(path);
  const h = createHash("sha256");
  h.update(buf);
  return h.digest("hex");
}

async function checkHostConfigEntryPresence(): Promise<CheckResult> {
  const cands = hostCandidates();
  let configured = 0;
  let presentCount = 0;
  const missingForPresent: string[] = [];
  for (const c of cands) {
    if (!(await fileExists(c))) continue;
    presentCount += 1;
    try {
      const raw = await readFile(c, "utf8");
      const parsed = JSON.parse(raw) as {
        mcpServers?: Record<string, unknown>;
      };
      if (parsed.mcpServers && "mnemonik" in parsed.mcpServers) {
        configured += 1;
      } else {
        missingForPresent.push(c);
      }
    } catch {
      missingForPresent.push(c);
    }
  }
  const pass = presentCount === 0 ? false : configured === presentCount;
  const detail =
    presentCount === 0
      ? "no host configs present; run `mnemonik-mcp install`"
      : pass
        ? `${configured}/${cands.length} hosts configured`
        : `${configured}/${cands.length} hosts configured; missing: ${missingForPresent.join(", ")}; repair: run \`mnemonik-mcp install\``;
  return { name: "host-config-entry-presence", pass, detail };
}

async function checkMcpHealth(): Promise<CheckResult> {
  try {
    const res = await fetch(HEALTH_URL);
    if (res.status !== 200) {
      return {
        name: "mcp-mnemonik-xyz-health",
        pass: false,
        detail: `HTTP ${res.status} from ${HEALTH_URL}; repair: check https://status.mnemonik.xyz or your network`,
      };
    }
    const body = (await res.json()) as { status?: string };
    if (body.status !== "ok") {
      return {
        name: "mcp-mnemonik-xyz-health",
        pass: false,
        detail: `unexpected body ${JSON.stringify(body)}; repair: check https://status.mnemonik.xyz`,
      };
    }
    return {
      name: "mcp-mnemonik-xyz-health",
      pass: true,
      detail: "200 OK",
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return {
      name: "mcp-mnemonik-xyz-health",
      pass: false,
      detail: `${msg}; repair: check your network or https://status.mnemonik.xyz`,
    };
  }
}

async function checkBinaryIntegrity(): Promise<CheckResult> {
  const bp = binaryPath();
  const mp = manifestPath();
  if (!(await fileExists(bp))) {
    return {
      name: "binary-integrity",
      pass: false,
      detail: `cached binary missing at ${bp}; repair: re-run any mnemonik-mcp subcommand to trigger lazy install`,
    };
  }
  if (!(await fileExists(mp))) {
    return {
      name: "binary-integrity",
      pass: false,
      detail: `manifest sidecar missing at ${mp}; repair: delete ${bp} and re-run any mnemonik-mcp subcommand`,
    };
  }
  try {
    const m = JSON.parse(await readFile(mp, "utf8")) as { sha256?: string };
    if (typeof m.sha256 !== "string") {
      return {
        name: "binary-integrity",
        pass: false,
        detail:
          "manifest.json lacks sha256 field; repair: delete cache dir and re-run",
      };
    }
    const onDisk = await sha256OfFile(bp);
    if (onDisk !== m.sha256) {
      return {
        name: "binary-integrity",
        pass: false,
        detail: `SHA256 mismatch on cached binary; expected ${m.sha256}, got ${onDisk}; repair: delete ${bp} and re-run`,
      };
    }
    return {
      name: "binary-integrity",
      pass: true,
      detail: `sha256 matches manifest (${m.sha256.slice(0, 12)}...)`,
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return {
      name: "binary-integrity",
      pass: false,
      detail: `manifest unreadable: ${msg}; repair: delete cache dir and re-run`,
    };
  }
}

function mnemonicConfigDir(): string {
  const override = process.env.MNEMONIC_CONFIG_DIR;
  if (override) return override;
  return join(homedir(), ".mnemonic");
}

async function checkLocalSqliteRw(): Promise<CheckResult> {
  const dir = mnemonicConfigDir();
  const probePath = join(dir, "doctor-rw-probe.tmp");
  try {
    await mkdir(dir, { recursive: true, mode: 0o700 });
    const { writeFile } = await import("node:fs/promises");
    const payload = `mnemonik-doctor-probe ${Date.now()}\n`;
    await writeFile(probePath, payload, { mode: 0o600 });
    const back = await readFile(probePath, "utf8");
    if (back !== payload) {
      throw new Error("read-back mismatch");
    }
    await unlink(probePath);
    return {
      name: "local-sqlite-rw",
      pass: true,
      detail: `${dir} writable`,
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    try {
      await unlink(probePath);
    } catch {
      // ignore cleanup error
    }
    return {
      name: "local-sqlite-rw",
      pass: false,
      detail: `${dir} not writable: ${msg}; repair: ensure ${dir} exists and is owned by you`,
    };
  }
}

async function checkIdentityAccessible(): Promise<CheckResult> {
  const idPath = join(mnemonicConfigDir(), "id.json");
  const idJsonPath = join(mnemonicConfigDir(), "identity.json");
  for (const p of [idPath, idJsonPath]) {
    if (await fileExists(p)) {
      try {
        const raw = await readFile(p, "utf8");
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        const hasPub =
          typeof parsed.publicKey === "string" ||
          typeof parsed.pubkey === "string" ||
          typeof parsed.public_key === "string";
        if (hasPub) {
          return {
            name: "identity-accessible",
            pass: true,
            detail: `pubkey present in ${p}`,
          };
        }
        return {
          name: "identity-accessible",
          pass: false,
          detail: `${p} lacks publicKey field; repair: re-run \`mnemonic init\``,
        };
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        return {
          name: "identity-accessible",
          pass: false,
          detail: `${p} unreadable: ${msg}; repair: re-run \`mnemonic init\``,
        };
      }
    }
  }
  return {
    name: "identity-accessible",
    pass: false,
    detail: `no identity file under ${mnemonicConfigDir()}; repair: run \`mnemonic init\``,
  };
}

async function checkTokenFileIntegrity(): Promise<CheckResult> {
  const tokenPath = join(mnemonicConfigDir(), "token.json");
  if (!(await fileExists(tokenPath))) {
    return {
      name: "token-file-integrity",
      pass: true,
      detail: "no cached token (will OAuth on first call)",
    };
  }
  try {
    const raw = await readFile(tokenPath, "utf8");
    const parsed = JSON.parse(raw) as { expires_at?: number | string };
    if (parsed.expires_at === undefined) {
      return {
        name: "token-file-integrity",
        pass: false,
        detail: `${tokenPath} missing expires_at; repair: delete the file and re-OAuth`,
      };
    }
    const exp =
      typeof parsed.expires_at === "string"
        ? Date.parse(parsed.expires_at) / 1000
        : parsed.expires_at;
    const nowSec = Math.floor(Date.now() / 1000);
    if (!Number.isFinite(exp)) {
      return {
        name: "token-file-integrity",
        pass: false,
        detail: `${tokenPath} expires_at unparseable; repair: delete the file and re-OAuth`,
      };
    }
    if ((exp as number) < nowSec) {
      // Soft warning per task spec — pass=true so doctor doesn't exit 1 on
      // an expired-but-otherwise-valid token; the agent will re-OAuth.
      return {
        name: "token-file-integrity",
        pass: true,
        detail: `token expired (will re-OAuth on next call)`,
      };
    }
    return {
      name: "token-file-integrity",
      pass: true,
      detail: `token valid until ${new Date((exp as number) * 1000).toISOString()}`,
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return {
      name: "token-file-integrity",
      pass: false,
      detail: `${tokenPath} malformed (${msg}); repair: delete the file and re-OAuth`,
    };
  }
}

export interface DoctorResult {
  exitCode: 0 | 1;
  checks: CheckResult[];
}

export async function doctor(): Promise<DoctorResult> {
  const checks: CheckResult[] = [];
  // Run sequentially to keep stdout ordered + avoid concurrent fs probes.
  checks.push(await checkHostConfigEntryPresence());
  checks.push(await checkMcpHealth());
  checks.push(await checkBinaryIntegrity());
  checks.push(await checkLocalSqliteRw());
  checks.push(await checkIdentityAccessible());
  checks.push(await checkTokenFileIntegrity());

  for (const c of checks) {
    const verdict = c.pass ? "pass" : "fail";
    process.stdout.write(`${c.name}: ${verdict}; ${c.detail}\n`);
  }
  const exitCode: 0 | 1 = checks.every((c) => c.pass) ? 0 : 1;
  return { exitCode, checks };
}

export const _internal = {
  checkHostConfigEntryPresence,
  checkMcpHealth,
  checkBinaryIntegrity,
  checkLocalSqliteRw,
  checkIdentityAccessible,
  checkTokenFileIntegrity,
};
