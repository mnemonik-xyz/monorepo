import { homedir as osHomedir, platform, arch } from "node:os";
import { join } from "node:path";

const PACKAGE_NAME = "@mnemonik-xyz/mcp";

/**
 * Honour HOME / USERPROFILE so tests can override $HOME without rewriting
 * the user's real home directory. `os.homedir()` is OS-syscall-backed and
 * ignores env overrides on macOS / Linux glibc.
 */
export function homedir(): string {
  return process.env.HOME ?? process.env.USERPROFILE ?? osHomedir();
}

export const REPO_OWNER = "mnemonik-xyz";
export const REPO_NAME = "monorepo";
// `gh attestation verify --signer-workflow` expects the full form
// `[host/]<owner>/<repo>/<path>/<to>/<workflow>` per `gh attestation verify --help`.
// A bare workflow path (`.github/workflows/release.yml`) silently fails
// verification with "Error: verifying with issuer sigstore.dev" because the
// signer SubjectAlternativeName the cert was issued with is the FULL URL form
// `https://github.com/<owner>/<repo>/.github/workflows/release.yml@<ref>`,
// not the bare path. Build the qualified form from REPO_OWNER + REPO_NAME so
// a future owner/repo rename only requires touching those constants.
export const SIGNER_WORKFLOW = `${REPO_OWNER}/${REPO_NAME}/.github/workflows/release.yml`;
export const RELEASE_BASE_URL =
  "https://github.com/mnemonik-xyz/monorepo/releases/download";

/**
 * Returns the directory under which the cached binary + manifest sidecar live.
 *
 * Override via MNEMONIK_MCP_CACHE_DIR for tests and dev. Otherwise honours
 * XDG_DATA_HOME, falling back to `~/.local/share/@mnemonik-xyz/mcp`.
 */
export function cacheDir(): string {
  const override = process.env.MNEMONIK_MCP_CACHE_DIR;
  if (override) return override;
  const xdg = process.env.XDG_DATA_HOME;
  const base = xdg && xdg.length > 0 ? xdg : join(homedir(), ".local", "share");
  return join(base, PACKAGE_NAME);
}

export function binaryPath(): string {
  return join(cacheDir(), "bin", "mnemonik-mcp");
}

export function manifestPath(): string {
  return join(cacheDir(), "manifest.json");
}

export type PlatformKey = "darwin-arm64" | "linux-x64";

export function detectPlatform(): PlatformKey {
  const p = platform();
  const a = arch();
  if (p === "darwin" && a === "arm64") return "darwin-arm64";
  if (p === "linux" && a === "x64") return "linux-x64";
  if (p === "darwin" && a === "x64") {
    throw new Error(
      "@mnemonik-xyz/mcp v1 ships Apple Silicon (arm64) only; " +
        "Intel Mac (x86_64) builds are not available. " +
        "Run on an arm64 Mac or wait for a later release.",
    );
  }
  throw new Error(
    `unsupported platform ${p}/${a}; @mnemonik-xyz/mcp v1 ships ` +
      `macOS (arm64) and Linux (x86_64) only`,
  );
}

/**
 * Returns the GitHub Releases base URL used by `ensureBinaryCached`.
 *
 * `MNEMONIK_MCP_RELEASE_BASE_URL` overrides the compiled-in default for tests
 * and self-hosted mirrors. The override MUST be `https://` so a local-malware
 * or compromised-shell-profile-injected env var cannot silently redirect the
 * download to an HTTP endpoint where a network MITM could substitute a
 * tampered tarball + matching SHA256SUMS (gh attestation verify is the last
 * gate, but defense-in-depth requires we don't lean on a single gate). The
 * `MNEMONIK_MCP_ALLOW_HTTP=1` escape hatch unblocks tests that point at a
 * local HTTP fixture — analogous to the Rust binary's `--allow-custom-endpoint`
 * pattern (tech-spec Decision 12). Per security-auditor round 1 SAR7-M2.
 */
export function releaseBaseUrl(): string {
  const override = process.env.MNEMONIK_MCP_RELEASE_BASE_URL;
  if (!override) return RELEASE_BASE_URL;
  let parsed: URL;
  try {
    parsed = new URL(override);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    throw new Error(`MNEMONIK_MCP_RELEASE_BASE_URL is not a valid URL: ${msg}`);
  }
  if (parsed.protocol === "https:") return override;
  if (
    parsed.protocol === "http:" &&
    process.env.MNEMONIK_MCP_ALLOW_HTTP === "1"
  ) {
    return override;
  }
  throw new Error(
    `MNEMONIK_MCP_RELEASE_BASE_URL must use https:// (got '${parsed.protocol}'); ` +
      `set MNEMONIK_MCP_ALLOW_HTTP=1 to permit http:// for local-only test fixtures`,
  );
}
