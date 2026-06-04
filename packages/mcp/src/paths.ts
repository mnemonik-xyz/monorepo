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
export const SIGNER_WORKFLOW = ".github/workflows/release.yml";
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

export type PlatformKey = "darwin-arm64" | "darwin-x64";

export function detectPlatform(): PlatformKey {
  const p = platform();
  const a = arch();
  if (p === "darwin" && a === "arm64") return "darwin-arm64";
  if (p === "darwin" && a === "x64") return "darwin-x64";
  throw new Error(
    `unsupported platform ${p}/${a}; @mnemonik-xyz/mcp v1 ships macOS only`,
  );
}

export function releaseBaseUrl(): string {
  return process.env.MNEMONIK_MCP_RELEASE_BASE_URL ?? RELEASE_BASE_URL;
}
