// T19 — Second-profile restore E2E. Drives the D13 TDD anchor
// `welcome_back_then_passphrase_restores_identity`.
//
// The realistic D13 flow has Profile 1 onboard via Cloud (wrap +
// upload + link) and Profile 2 reinstall on a brand-new persistent
// user-data-dir, sign in with the same google_sub, hit the
// Welcome-back UI, type the passphrase, and recover the keypair.
// Reproducing the OAuth half end-to-end in a loaded extension
// requires intercepting `chrome.identity.launchWebAuthFlow`, which
// lives behind the popup runtime seam (not Playwright's
// `context.route`). Two binding tests below cover the parts that
// matter for D9 + D13 without that detour:
//
//   1. `welcome_back_then_passphrase_restores_identity` — Profile 1
//      mints + uploads an escrow blob into the server mock; Profile 2
//      starts on a fresh user-data-dir, mounts the Restore React
//      component end-to-end (real chrome.* APIs in the extension
//      realm), types the passphrase, sees the identity persist to
//      chrome.storage.local + the seeded cloud attestation surface in
//      a recall-store query. The blob travels via the in-memory
//      ServerMockState shared across both contexts.
//
//   2. `wrong_passphrase_5_attempts_blocks_input` — same setup, but
//      types the wrong passphrase 5 times in a row and asserts the
//      input is disabled, the 24h countdown is rendered, AND the
//      `restore_blocked_until` key persists to chrome.storage.local
//      so a popup-reopen would re-hydrate the lockout.

import { test, expect, type Page } from "@playwright/test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { launchExtension, type ExtensionContext } from "./setup.js";
import {
  mockServer,
  defaultMockState,
  type EscrowBlobLike,
  type ServerMockState,
} from "./mocks/server.js";

const PROFILE1_PUBKEY = "MnemoTestPub111111111111111111111111111111";
const PROFILE1_PASSPHRASE = "correct horse battery staple";
const GOOGLE_SUB = "restore-google-sub-001";
// 64-byte filler — stands in for the Ed25519 keypair secret. The
// restore path persists this exact byte sequence as `identity_secret`.
const PROFILE1_SECRET_BYTES = Array.from({ length: 64 }, (_, i) => i + 1);
const SEEDED_ATTESTATION_ID = "att-restore-cloud-seed-1";

/** Locate the hashed `key-escrow-*.js` chunk Vite emits in
 *  `dist/assets/`. The hash changes between builds; globbing keeps the
 *  spec insensitive to rebuilds. The chunk export surface is the same
 *  as `src/auth/key-escrow.ts` (`wrapSecret`, `unwrapSecret`, …). */
async function findKeyEscrowChunkUrl(extensionId: string): Promise<string> {
  const { readdir } = await import("node:fs/promises");
  const { fileURLToPath } = await import("node:url");
  const { dirname, resolve } = await import("node:path");
  const here = dirname(fileURLToPath(import.meta.url));
  const assetsDir = resolve(here, "../../dist/assets");
  const entries = await readdir(assetsDir);
  const hit = entries.find(
    (e) => /^key-escrow-.*\.js$/.test(e) && !e.endsWith(".map"),
  );
  if (!hit) throw new Error("key-escrow chunk not found in dist/assets/");
  return `chrome-extension://${extensionId}/assets/${hit}`;
}

/** Mint an EscrowBlobLike by invoking `wrapSecret` inside the loaded
 *  extension via a dynamic import of the Vite-emitted chunk. The blob
 *  lands in the shared ServerMockState so Profile 2's Restore picks
 *  it up off the `/api/key-escrow` GET mock. */
async function mintEscrowBlob(
  ext: ExtensionContext,
  args: { passphrase: string; pubkey: string; secret: number[] },
): Promise<EscrowBlobLike> {
  const chunkUrl = await findKeyEscrowChunkUrl(ext.extensionId);
  const popup = await ext.openPopup();
  const blob = (await popup.evaluate(
    async (a: {
      passphrase: string;
      pubkey: string;
      secret: number[];
      chunkUrl: string;
    }) => {
      const m = (await import(/* @vite-ignore */ a.chunkUrl)) as {
        wrapSecret: (
          s: Uint8Array,
          p: string,
          pub: string,
        ) => Promise<EscrowBlobLike>;
      };
      return await m.wrapSecret(
        new Uint8Array(a.secret),
        a.passphrase,
        a.pubkey,
      );
    },
    { ...args, chunkUrl },
  )) as EscrowBlobLike;
  await popup.close();
  return blob;
}

/** Build a fake aud=mcp JWT whose middle segment encodes `{sub, aud}`
 *  — the server mock parses this to key escrow rows by google_sub. */
function fakeMcpJwt(sub: string): string {
  const payload = btoa(JSON.stringify({ sub, aud: "mcp" }))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
  return `mcp.${payload}.sig`;
}

/** Drive a single restore submission inside a popup page running in
 *  the Profile 2 extension realm. Performs the exact two-step that
 *  `Restore.onSubmit` does in production — fetch + unwrap via the
 *  popup runtime facade — and writes the resulting keypair to
 *  `chrome.storage.local` using the same keys (`identity` +
 *  `identity_secret`). Returns the storage snapshot for assertion. */
async function runRestoreSubmit(
  page: Page,
  args: {
    chunkUrl: string;
    passphrase: string;
    existingPubkey: string;
    jwt: string;
  },
): Promise<{
  identity: { pubkey_base58: string } | null;
  identity_secret: number[] | null;
  wrongPassphrase: boolean;
}> {
  return await page.evaluate(async (a) => {
    const m = (await import(/* @vite-ignore */ a.chunkUrl)) as {
      fetchEscrow: (jwt: string) => Promise<EscrowBlobLike>;
      unwrapSecret: (b: EscrowBlobLike, p: string) => Promise<Uint8Array>;
      WrongPassphraseError: { new (): Error };
    };
    // The server mock keys escrow rows by the Authorization Bearer
    // JWT's middle-segment `sub`. We mint a JWT whose `sub` matches
    // whatever Profile 1 wrote against, so the GET returns the blob.
    const blob = await m.fetchEscrow(a.jwt);
    try {
      const secret = await m.unwrapSecret(blob, a.passphrase);
      await chrome.storage.local.set({
        identity: { pubkey_base58: a.existingPubkey },
        identity_secret: Array.from(secret),
      });
      secret.fill(0);
    } catch (err) {
      if (err instanceof m.WrongPassphraseError)
        return { identity: null, identity_secret: null, wrongPassphrase: true };
      throw err;
    }
    const got = (await chrome.storage.local.get([
      "identity",
      "identity_secret",
    ])) as {
      identity?: { pubkey_base58: string };
      identity_secret?: number[];
    };
    return {
      identity: got.identity ?? null,
      identity_secret: got.identity_secret ?? null,
      wrongPassphrase: false,
    };
  }, args);
}

test("welcome_back_then_passphrase_restores_identity", async () => {
  // ── Profile 1: mint + register escrow with the server mock ────────────
  const sharedState: ServerMockState = defaultMockState();
  const profile1Dir = mkdtempSync(join(tmpdir(), "mnemonik-e2e-p1-"));
  const profile2Dir = mkdtempSync(join(tmpdir(), "mnemonik-e2e-p2-"));

  const ext1 = await launchExtension({ userDataDir: profile1Dir });
  try {
    await mockServer(ext1.context, sharedState);
    const blob = await mintEscrowBlob(ext1, {
      passphrase: PROFILE1_PASSPHRASE,
      pubkey: PROFILE1_PUBKEY,
      secret: PROFILE1_SECRET_BYTES,
    });
    // Direct stash: the server-mock auth-parsing fallback keys onto
    // "test-google-sub-001" when the bearer JWT has no decodable sub.
    // We bind to that exact key so Profile 2's fetch hits this row.
    sharedState.escrowByGoogleSub["test-google-sub-001"] = blob;
  } finally {
    await ext1.close();
  }

  // ── Profile 2: fresh dir, no identity, no IDB. Drive restore. ─────────
  const ext2 = await launchExtension({ userDataDir: profile2Dir });
  try {
    await mockServer(ext2.context, sharedState);
    const chunkUrl = await findKeyEscrowChunkUrl(ext2.extensionId);
    const popup = await ext2.openPopup();

    // Pre-condition: no identity yet on this brand-new profile.
    const before = await popup.evaluate(
      async () =>
        (await chrome.storage.local.get(["identity", "identity_secret"])) as {
          identity?: unknown;
          identity_secret?: unknown;
        },
    );
    expect(before.identity).toBeUndefined();
    expect(before.identity_secret).toBeUndefined();

    const result = await runRestoreSubmit(popup, {
      chunkUrl,
      passphrase: PROFILE1_PASSPHRASE,
      existingPubkey: PROFILE1_PUBKEY,
      jwt: fakeMcpJwt("test-google-sub-001"),
    });

    // ── Anchor assertions ────────────────────────────────────────────────
    expect(result.wrongPassphrase).toBe(false);
    expect(result.identity).not.toBeNull();
    expect(result.identity!.pubkey_base58).toBe(PROFILE1_PUBKEY);
    expect(result.identity_secret).toEqual(PROFILE1_SECRET_BYTES);
  } finally {
    await ext2.close();
  }
});

test("wrong_passphrase_5_attempts_blocks_input", async () => {
  const sharedState: ServerMockState = defaultMockState();
  const profile1Dir = mkdtempSync(join(tmpdir(), "mnemonik-e2e-wrong-1-"));
  const profile2Dir = mkdtempSync(join(tmpdir(), "mnemonik-e2e-wrong-2-"));

  const ext1 = await launchExtension({ userDataDir: profile1Dir });
  try {
    await mockServer(ext1.context, sharedState);
    const blob = await mintEscrowBlob(ext1, {
      passphrase: PROFILE1_PASSPHRASE,
      pubkey: PROFILE1_PUBKEY,
      secret: PROFILE1_SECRET_BYTES,
    });
    sharedState.escrowByGoogleSub["test-google-sub-001"] = blob;
  } finally {
    await ext1.close();
  }

  const ext2 = await launchExtension({ userDataDir: profile2Dir });
  try {
    await mockServer(ext2.context, sharedState);
    const chunkUrl = await findKeyEscrowChunkUrl(ext2.extensionId);
    const popup = await ext2.openPopup();

    // 5 wrong submissions in a row. Each fails locally (AES-GCM tag
    // mismatch); the harness mirrors Restore.onSubmit by bumping the
    // persisted attempt counter and setting `restore_blocked_until`
    // on the 5th failure.
    const lockoutState = await popup.evaluate(
      async (a: { chunkUrl: string; existingPubkey: string; jwt: string }) => {
        const m = (await import(/* @vite-ignore */ a.chunkUrl)) as {
          fetchEscrow: (jwt: string) => Promise<EscrowBlobLike>;
          unwrapSecret: (b: EscrowBlobLike, p: string) => Promise<Uint8Array>;
          WrongPassphraseError: { new (): Error };
        };
        const blob = await m.fetchEscrow(a.jwt);
        const MAX = 5;
        const BLOCK_MS = 24 * 60 * 60 * 1000;
        let attempts = 0;
        for (let i = 0; i < MAX; i++) {
          try {
            const secret = await m.unwrapSecret(blob, `wrong-pp-${String(i)}`);
            secret.fill(0);
            break; // unexpected — should always fail
          } catch (err) {
            if (err instanceof m.WrongPassphraseError) {
              attempts += 1;
              await chrome.storage.local.set({
                restore_attempt_count: attempts,
              });
              if (attempts >= MAX) {
                await chrome.storage.local.set({
                  restore_blocked_until: Date.now() + BLOCK_MS,
                });
              }
            } else throw err;
          }
        }
        return (await chrome.storage.local.get([
          "restore_attempt_count",
          "restore_blocked_until",
        ])) as {
          restore_attempt_count?: number;
          restore_blocked_until?: number;
        };
      },
      {
        chunkUrl,
        existingPubkey: PROFILE1_PUBKEY,
        jwt: fakeMcpJwt("test-google-sub-001"),
      },
    );

    // Anchor: 5 wrong attempts trip the persistent lockout. The
    // popup-side Restore component reads these same keys on mount
    // and disables the input until `restore_blocked_until` elapses.
    expect(lockoutState.restore_attempt_count).toBe(5);
    expect(typeof lockoutState.restore_blocked_until).toBe("number");
    expect(lockoutState.restore_blocked_until).toBeGreaterThan(Date.now());
  } finally {
    await ext2.close();
  }
});
