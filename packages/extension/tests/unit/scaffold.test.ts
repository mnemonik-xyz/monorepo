import { describe, it, expect } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

// T01 TDD anchor: `dist_has_valid_manifest`.
// Drives D11 (enumerated host_permissions) and the MV3 baseline.
// Reads the source manifest at the package root — the build copies it to
// dist/manifest.json verbatim via @crxjs/vite-plugin, so validating the source
// is equivalent to validating the build output without requiring `vite build`
// in the unit-test path.

const manifestPath = resolve(__dirname, "../../manifest.json");

describe("scaffold · manifest.json", () => {
  it("exists at the package root", () => {
    expect(existsSync(manifestPath)).toBe(true);
  });

  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

  it("declares manifest_version 3", () => {
    expect(manifest.manifest_version).toBe(3);
  });

  it("has the expected permission set", () => {
    expect(manifest.permissions).toEqual(
      expect.arrayContaining([
        "storage",
        "identity",
        "contextMenus",
        "activeTab",
        "clipboardWrite",
        "alarms",
      ]),
    );
  });

  it("declares only enumerated AI-chat host_permissions per D11", () => {
    // T07 added chatgpt.com; T08 added claude.ai; T09 appends
    // gemini.google.com. `<all_urls>` is explicitly forbidden — generic
    // page capture flows through `activeTab` per D8/D11. Uses
    // `arrayContaining` so co-running adapter PRs can land in any
    // order — resolving the conflict only requires alphabetising the
    // host_permissions array.
    expect(manifest.host_permissions).toEqual(
      expect.arrayContaining([
        "https://chatgpt.com/*",
        "https://claude.ai/*",
        "https://gemini.google.com/*",
      ]),
    );
    expect(manifest.host_permissions).not.toContain("<all_urls>");
    for (const entry of manifest.host_permissions) {
      expect(entry).toMatch(/^https:\/\/[a-z0-9.-]+\/\*$/);
    }
  });

  it("registers popup, options, and a service-worker entry point", () => {
    expect(manifest.action.default_popup).toMatch(/popup\/index\.html$/);
    expect(manifest.options_ui.page).toMatch(/options\/index\.html$/);
    expect(manifest.background.service_worker).toMatch(
      /service-worker\.ts$/,
    );
    expect(manifest.background.type).toBe("module");
  });

  it("registers _execute_action and recall-overlay commands", () => {
    expect(manifest.commands).toBeDefined();
    expect(manifest.commands._execute_action).toBeDefined();
    expect(manifest.commands["recall-overlay"]).toBeDefined();
  });
});
