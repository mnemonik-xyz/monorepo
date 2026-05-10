import { describe, it, expect } from "vitest";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const sourceManifestPath = resolve(__dirname, "../../manifest.json");
const distManifestPath = resolve(__dirname, "../../dist/manifest.json");

describe("MV3 manifest", () => {
  const manifest = JSON.parse(readFileSync(sourceManifestPath, "utf8"));

  it("dist_has_valid_manifest: parses and is MV3", () => {
    expect(manifest.manifest_version).toBe(3);
    expect(manifest.name).toBe("Mnemonik");
    expect(manifest.version).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it("declares the expected enumerated permissions (D11)", () => {
    const expected = [
      "storage",
      "identity",
      "contextMenus",
      "activeTab",
      "clipboardWrite",
      "alarms",
    ];
    expect(manifest.permissions).toEqual(expected);
  });

  it("does not request <all_urls> or any host permission yet (D11)", () => {
    expect(manifest.host_permissions).toEqual([]);
    expect(JSON.stringify(manifest)).not.toContain("<all_urls>");
  });

  it("wires popup, options, background module, and hotkey command", () => {
    expect(manifest.action.default_popup).toBe("src/popup/index.html");
    expect(manifest.options_page).toBe("src/options/index.html");
    expect(manifest.background.service_worker).toBe(
      "src/background/service-worker.ts",
    );
    expect(manifest.background.type).toBe("module");
    expect(manifest.commands["recall-overlay"]).toBeDefined();
    expect(manifest.commands._execute_action).toBeDefined();
  });
});

describe("dist/manifest.json (post-build)", () => {
  const hasDist = existsSync(distManifestPath);

  it.runIf(hasDist)("dist_has_valid_manifest: parses, MV3, same permissions", () => {
    const dist = JSON.parse(readFileSync(distManifestPath, "utf8"));
    expect(dist.manifest_version).toBe(3);
    expect(dist.permissions).toEqual([
      "storage",
      "identity",
      "contextMenus",
      "activeTab",
      "clipboardWrite",
      "alarms",
    ]);
    expect(dist.host_permissions).toEqual([]);
  });
});
