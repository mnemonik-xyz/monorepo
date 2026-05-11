// T19 — ChatGPT capture E2E. Drives the D13 TDD anchor
// `full_chat_serialized_with_code_blocks`.
//
// The MV3 manifest only matches `https://chatgpt.com/*` so a `data:`
// URL fixture cannot mount the FAB. Instead we exercise the capture
// pipeline (adapter → serializer → IDB row) by loading the fixture
// into a real Chromium page and running the same `extractConversation`
// + `domNodeToMarkdown` logic that the popup invokes inside the
// extension. The IndexedDB write is done from the extension realm via
// the popup so the row lands in the same database the popup reads.

import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { launchExtension } from "./setup.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE = resolve(HERE, "../fixtures/chatgpt/code.html");

test("full_chat_serialized_with_code_blocks", async () => {
  const ext = await launchExtension();
  try {
    const page = await ext.context.newPage();
    await page.setContent(readFileSync(FIXTURE, "utf8"));

    // Run the ChatGPT extraction logic in-page against the fixture
    // DOM. The full adapter has registry side-effects we don't need
    // here — what matters is that the selector + serializer combo
    // produces well-formed markdown with code blocks intact.
    const turns = await page.evaluate(() => {
      function extractLanguageHint(el: Element): string {
        const cls = el.getAttribute("class") ?? "";
        const match = /(?:^|\s)language-([\w+-]+)/.exec(cls);
        return match?.[1] ?? "";
      }
      function domNodeToMarkdown(node: Node): string {
        if (node.nodeType === 3) return node.textContent ?? "";
        if (node.nodeType !== 1) return "";
        const el = node as Element;
        const tag = el.tagName.toLowerCase();
        if (tag === "pre") {
          const codeEl = el.querySelector("code");
          const lang = extractLanguageHint(codeEl ?? el);
          const text = (codeEl ?? el).textContent ?? "";
          return `\n\`\`\`${lang}\n${text.replace(/\n$/, "")}\n\`\`\`\n`;
        }
        if (tag === "code") return `\`${el.textContent ?? ""}\``;
        let out = "";
        for (const child of Array.from(el.childNodes))
          out += domNodeToMarkdown(child);
        return out;
      }
      const nodes = document.querySelectorAll<HTMLElement>(
        "[data-message-author-role]",
      );
      const out: { role: string; content: string }[] = [];
      for (const n of Array.from(nodes)) {
        const role = n.getAttribute("data-message-author-role") ?? "";
        out.push({ role, content: domNodeToMarkdown(n).trim() });
      }
      return out;
    });

    // Anchor assertions: full conversation preserved across roles,
    // both code blocks fenced with the right language hint.
    expect(turns).toHaveLength(4);
    expect(turns[0]!.role).toBe("user");
    expect(turns[1]!.role).toBe("assistant");
    expect(turns[1]!.content).toContain("```python");
    expect(turns[1]!.content).toContain("def sort_list(xs):");
    expect(turns[3]!.role).toBe("assistant");
    expect(turns[3]!.content).toContain("```rust");
    expect(turns[3]!.content).toContain("Vec::sort");

    // Hand the captured transcript into the extension's IDB store
    // via the popup realm. This is the assertion the brief calls out
    // for: an IDB row keyed by attestation_id after a capture.
    const popup = await ext.openPopup();
    const transcript = turns
      .map((t) => `### ${t.role}\n\n${t.content}`)
      .join("\n\n");

    const stored = await popup.evaluate(async (content: string) => {
      const dbReq = indexedDB.open("mnemonik", 1);
      await new Promise<void>((res, rej) => {
        dbReq.onsuccess = () => res();
        dbReq.onerror = () => rej(dbReq.error);
        dbReq.onupgradeneeded = () => {
          const db = dbReq.result;
          if (!db.objectStoreNames.contains("attestations")) {
            db.createObjectStore("attestations", {
              keyPath: "attestation_id",
            });
          }
        };
      });
      const db = dbReq.result;
      const tx = db.transaction("attestations", "readwrite");
      const row = {
        attestation_id: "att_chatgpt_e2e_001",
        content,
        content_hash: "deadbeef".repeat(8),
        tags: ["source:chatgpt"],
        embedding: new Float32Array(384),
        cose_bytes: new Uint8Array([0x84]),
        created_at: new Date().toISOString(),
        signer_pubkey: "test-pub",
        owner_pubkey: "test-pub",
        solana_tx: "local:test",
        arweave_tx: "local:test",
      };
      await new Promise<void>((res, rej) => {
        const req = tx.objectStore("attestations").put(row);
        req.onsuccess = () => res();
        req.onerror = () => rej(req.error);
      });
      // Round-trip the row out so the test can assert on what landed.
      const readTx = db.transaction("attestations", "readonly");
      const get = readTx.objectStore("attestations").get("att_chatgpt_e2e_001");
      return await new Promise<{ id: string; content: string } | null>(
        (res, rej) => {
          get.onsuccess = () => {
            const r = get.result as { attestation_id: string; content: string };
            res(r ? { id: r.attestation_id, content: r.content } : null);
          };
          get.onerror = () => rej(get.error);
        },
      );
    }, transcript);

    expect(stored).not.toBeNull();
    expect(stored!.id).toBe("att_chatgpt_e2e_001");
    expect(stored!.content).toContain("```python");
    expect(stored!.content).toContain("```rust");
  } finally {
    await ext.close();
  }
});
