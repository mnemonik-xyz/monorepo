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
    // T10 round-2: `clipboardWrite` dropped (least-privilege; T13 adds
    // it back when the recall overlay's "copy hash" gesture lands).
    // `scripting` added so T13 can use `chrome.scripting.executeScript`
    // on the generic-page recall-overlay path (activeTab-gated).
    expect(manifest.permissions).toEqual(
      expect.arrayContaining([
        "storage",
        "identity",
        "contextMenus",
        "activeTab",
        "scripting",
        "alarms",
      ])
    );
    expect(manifest.permissions).not.toContain("clipboardWrite");
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
      ])
    );
    expect(manifest.host_permissions).not.toContain("<all_urls>");
    for (const entry of manifest.host_permissions) {
      expect(entry).toMatch(/^https:\/\/[a-z0-9.-]+\/\*$/);
    }
  });

  it("registers popup, options, and a service-worker entry point", () => {
    expect(manifest.action.default_popup).toMatch(/popup\/index\.html$/);
    expect(manifest.options_ui.page).toMatch(/options\/index\.html$/);
    expect(manifest.background.service_worker).toMatch(/service-worker\.ts$/);
    expect(manifest.background.type).toBe("module");
  });

  it("registers _execute_action and recall-overlay commands", () => {
    expect(manifest.commands).toBeDefined();
    expect(manifest.commands._execute_action).toBeDefined();
    expect(manifest.commands["recall-overlay"]).toBeDefined();
  });

  it("registers content_scripts for each AI-chat origin with idle run_at", () => {
    expect(Array.isArray(manifest.content_scripts)).toBe(true);
    expect(manifest.content_scripts.length).toBeGreaterThanOrEqual(3);
    const matches = manifest.content_scripts.flatMap(
      (c: { matches: string[] }) => c.matches
    );
    expect(matches).toEqual(
      expect.arrayContaining([
        "https://chatgpt.com/*",
        "https://claude.ai/*",
        "https://gemini.google.com/*",
      ])
    );
    expect(matches).not.toContain("<all_urls>");
    for (const cs of manifest.content_scripts as Array<{
      js?: string[];
      run_at?: string;
    }>) {
      expect(Array.isArray(cs.js) && (cs.js?.length ?? 0) > 0).toBe(true);
      expect(
        cs.run_at === "document_idle" || cs.run_at === "document_end"
      ).toBe(true);
    }
  });

  it("scopes web_accessible_resources to enumerated origins (no wildcards)", () => {
    expect(Array.isArray(manifest.web_accessible_resources)).toBe(true);
    expect(manifest.web_accessible_resources.length).toBeGreaterThanOrEqual(1);
    for (const war of manifest.web_accessible_resources as Array<{
      resources: string[];
      matches: string[];
    }>) {
      expect(war.resources.length).toBeGreaterThan(0);
      expect(war.matches.length).toBeGreaterThan(0);
      expect(war.matches).not.toContain("<all_urls>");
      for (const m of war.matches) {
        expect(m).toMatch(/^https:\/\/[a-z0-9.-]+\/\*$/);
      }
      // T10 round-2 (T10-S-06): no `src/assets/*` wildcard — list
      // each icon file explicitly so future additions are reviewed.
      expect(war.resources).not.toContain("src/assets/*");
    }
  });

  it("declares a strict CSP with no unsafe-eval and no remote script sources", () => {
    expect(manifest.content_security_policy).toBeDefined();
    const csp = manifest.content_security_policy.extension_pages as string;
    expect(typeof csp).toBe("string");
    expect(csp.length).toBeGreaterThan(0);
    // D12 / T10-S-04: wasm-unsafe-eval is allowed; bare unsafe-eval is not.
    expect(csp).toContain("'wasm-unsafe-eval'");
    expect(csp).not.toMatch(/(?<!wasm-)unsafe-eval/);
    expect(csp).not.toMatch(/unsafe-inline/);
    expect(csp).toContain("script-src 'self'");
    // T10-S-04: worker-src 'self' future-proofs the embedder Web Worker.
    expect(csp).toContain("worker-src 'self'");
    // T10-S-03: regional HF CDN coverage so cdn-lfs-us-1.huggingface.co
    // (and friends) aren't blocked at model-download time.
    expect(csp).toContain("https://*.huggingface.co");
  });
});
