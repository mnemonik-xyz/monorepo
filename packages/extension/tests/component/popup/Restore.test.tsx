// Restore-component tests (T17). Binds the third TDD anchor:
//
//   5_wrong_attempts_blocks_input — five wrong passphrases in a row
//   disable the input and surface a 24-hour countdown that survives a
//   popup re-mount via persisted `restore_blocked_until` storage.
//
// We hand the component a `fastWrap`-built blob (matching the real wire
// shape but using reduced Argon2id parameters) so each unwrap round-trip
// stays under ~30ms. The fetch impl is a `vi.fn` returning the blob;
// no real network is touched. Storage is a Map-backed stub injected via
// the `storage` prop so we can observe the persisted block timestamp.

import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  Restore,
  MAX_WRONG_ATTEMPTS,
  BLOCK_WINDOW_MS,
  RESTORE_BLOCKED_KEY,
  IDENTITY_STORAGE_KEY,
  IDENTITY_SECRET_STORAGE_KEY,
  formatCountdown,
  truncatePubkey,
} from "../../../src/popup/onboarding/Restore.js";
import {
  ARGON2ID_SALT_LENGTH,
  AES_GCM_NONCE_LENGTH,
  KDF_IDENTIFIER,
  deriveKey,
  type EscrowBlob,
} from "../../../src/auth/key-escrow.js";

// ── Helpers ─────────────────────────────────────────────────────────────────

const FAST_PARAMS = { memoryCostKib: 8, timeCost: 1, parallelism: 1 };

async function fastWrap(
  secret: Uint8Array,
  passphrase: string,
  pubkey: string,
): Promise<EscrowBlob> {
  const salt = crypto.getRandomValues(new Uint8Array(ARGON2ID_SALT_LENGTH));
  const nonce = crypto.getRandomValues(new Uint8Array(AES_GCM_NONCE_LENGTH));
  const key = await deriveKey(passphrase, salt, FAST_PARAMS);
  const ct = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: nonce },
    key,
    secret,
  );
  const b64 = (b: Uint8Array): string =>
    globalThis.btoa(String.fromCharCode(...b));
  return {
    ciphertext_b64: b64(new Uint8Array(ct)),
    nonce_b64: b64(nonce),
    kdf: KDF_IDENTIFIER,
    kdf_params: JSON.stringify({
      salt_b64: b64(salt),
      m: FAST_PARAMS.memoryCostKib,
      t: FAST_PARAMS.timeCost,
      p: FAST_PARAMS.parallelism,
    }),
    pubkey_base58: pubkey,
  };
}

function makeStorage(): {
  store: Map<string, unknown>;
  adapter: {
    get(keys: string[]): Promise<Record<string, unknown>>;
    set(items: Record<string, unknown>): Promise<void>;
  };
} {
  const store = new Map<string, unknown>();
  return {
    store,
    adapter: {
      async get(keys: string[]) {
        const out: Record<string, unknown> = {};
        for (const k of keys) {
          if (store.has(k)) out[k] = store.get(k);
        }
        return out;
      },
      async set(items: Record<string, unknown>) {
        for (const [k, v] of Object.entries(items)) store.set(k, v);
      },
    },
  };
}

function fetchReturning(blob: EscrowBlob): typeof fetch {
  return (async () =>
    new Response(JSON.stringify(blob), {
      status: 200,
      headers: { "content-type": "application/json" },
    })) as typeof fetch;
}

// ── 5_wrong_attempts_blocks_input — TDD anchor #3 ───────────────────────────

describe("Restore — 5_wrong_attempts_blocks_input (TDD anchor)", () => {
  it("five wrong passphrases disable the input and persist the block", async () => {
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const blob = await fastWrap(secret, "correct-passphrase-A", "PubABC123");
    const { store, adapter } = makeStorage();
    let nowMs = 1_700_000_000_000;

    render(
      <Restore
        jwt="JWT.PLACEHOLDER"
        existingPubkey="PubABC123"
        onComplete={() => undefined}
        serverOrigin="https://mc.test"
        fetchImpl={fetchReturning(blob)}
        now={() => nowMs}
        storage={adapter}
      />,
    );

    const input = (await screen.findByTestId(
      "restore-pp-input",
    )) as HTMLInputElement;
    const submit = screen.getByRole("button", { name: /^Restore/ });

    expect(input.disabled).toBe(false);

    // Submit the wrong passphrase MAX_WRONG_ATTEMPTS times.
    for (let i = 1; i <= MAX_WRONG_ATTEMPTS; i++) {
      fireEvent.change(input, {
        target: { value: `bogus-attempt-${String(i)}-padding` },
      });
      fireEvent.click(submit);
      // Wait for the per-attempt error message OR the final block message.
      // eslint-disable-next-line no-await-in-loop
      await waitFor(() => {
        const err = screen.queryByTestId("restore-error");
        const block = screen.queryByTestId("restore-block-countdown");
        expect(err !== null || block !== null).toBe(true);
      });
    }

    // After the 5th wrong attempt, the block must be active:
    //   - input is disabled,
    //   - countdown is rendered,
    //   - persisted `restore_blocked_until` is set ~24h in the future.
    await waitFor(() => {
      expect(input.disabled).toBe(true);
    });
    expect(screen.getByTestId("restore-block-countdown").textContent).toMatch(
      /Restore locked/,
    );
    const persisted = store.get(RESTORE_BLOCKED_KEY);
    expect(typeof persisted).toBe("number");
    expect((persisted as number) - nowMs).toBeGreaterThanOrEqual(
      BLOCK_WINDOW_MS - 1000,
    );
    expect((persisted as number) - nowMs).toBeLessThanOrEqual(
      BLOCK_WINDOW_MS + 1000,
    );
  });
});

// ── Happy path ──────────────────────────────────────────────────────────────

describe("Restore — happy path", () => {
  it("correct passphrase persists keypair and fires onComplete", async () => {
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const blob = await fastWrap(secret, "correct-passphrase-A", "PubHappy");
    const { store, adapter } = makeStorage();
    let completed = false;

    render(
      <Restore
        jwt="JWT.PLACEHOLDER"
        existingPubkey="PubHappy"
        onComplete={() => {
          completed = true;
        }}
        serverOrigin="https://mc.test"
        fetchImpl={fetchReturning(blob)}
        storage={adapter}
      />,
    );

    const input = (await screen.findByTestId(
      "restore-pp-input",
    )) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "correct-passphrase-A" } });
    fireEvent.click(screen.getByRole("button", { name: /^Restore/ }));

    await waitFor(() => {
      expect(completed).toBe(true);
    });
    const identity = store.get(IDENTITY_STORAGE_KEY) as
      | { pubkey_base58: string }
      | undefined;
    const secretArr = store.get(IDENTITY_SECRET_STORAGE_KEY) as
      | number[]
      | undefined;
    expect(identity).toBeDefined();
    expect(identity?.pubkey_base58).toBe("PubHappy");
    expect(secretArr).toBeDefined();
    expect(secretArr!.length).toBe(secret.length);
    expect(Uint8Array.from(secretArr!)).toEqual(secret);
  });
});

// ── 429 rate-limit surfacing ────────────────────────────────────────────────

describe("Restore — 429 surfaces typed countdown", () => {
  it("EscrowRateLimitError renders the retry-after copy", async () => {
    const { adapter } = makeStorage();
    const fetchImpl = (async () =>
      new Response(JSON.stringify({ error: "rate_limited" }), {
        status: 429,
        headers: { "retry-after": "300" },
      })) as typeof fetch;

    render(
      <Restore
        jwt="JWT.PLACEHOLDER"
        existingPubkey="PubRL"
        onComplete={() => undefined}
        serverOrigin="https://mc.test"
        fetchImpl={fetchImpl}
        now={() => 1_700_000_000_000}
        storage={adapter}
      />,
    );

    const input = (await screen.findByTestId(
      "restore-pp-input",
    )) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "anything-long-enough-here" } });
    fireEvent.click(screen.getByRole("button", { name: /^Restore/ }));

    await waitFor(() => {
      expect(
        screen.getByTestId("restore-rate-limit-countdown"),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByTestId("restore-rate-limit-countdown").textContent,
    ).toMatch(/Server limited fetches/);
    // Input must be disabled while the rate-limit window is active.
    expect(input.disabled).toBe(true);
  });
});

// ── Persisted block survives popup re-mount ─────────────────────────────────

describe("Restore — persisted block survives mount", () => {
  it("a future restore_blocked_until disables the input on first paint", async () => {
    const { adapter, store } = makeStorage();
    const nowMs = 1_700_000_000_000;
    store.set(RESTORE_BLOCKED_KEY, nowMs + 60_000);

    render(
      <Restore
        jwt="JWT.PLACEHOLDER"
        existingPubkey="PubPersist"
        onComplete={() => undefined}
        serverOrigin="https://mc.test"
        fetchImpl={fetchReturning({
          ciphertext_b64: "AAA=",
          nonce_b64: "AAAAAAAAAAAAAAAA",
          kdf: KDF_IDENTIFIER,
          kdf_params: JSON.stringify({
            salt_b64: "AAAA",
            m: 8,
            t: 1,
            p: 1,
          }),
          pubkey_base58: "PubPersist",
        })}
        now={() => nowMs}
        storage={adapter}
      />,
    );

    const input = (await screen.findByTestId(
      "restore-pp-input",
    )) as HTMLInputElement;
    await waitFor(() => {
      expect(input.disabled).toBe(true);
    });
    expect(screen.getByTestId("restore-block-countdown")).toBeInTheDocument();
  });
});

// ── Forgot-passphrase modal ─────────────────────────────────────────────────

describe("Restore — forgot-passphrase modal", () => {
  it("opens with two stub-disabled options", async () => {
    const { adapter } = makeStorage();
    render(
      <Restore
        jwt="JWT.PLACEHOLDER"
        existingPubkey="PubForgot"
        onComplete={() => undefined}
        serverOrigin="https://mc.test"
        fetchImpl={fetchReturning({
          ciphertext_b64: "AAA=",
          nonce_b64: "AAAAAAAAAAAAAAAA",
          kdf: KDF_IDENTIFIER,
          kdf_params: JSON.stringify({
            salt_b64: "AAAA",
            m: 8,
            t: 1,
            p: 1,
          }),
          pubkey_base58: "PubForgot",
        })}
        storage={adapter}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: /Forgot passphrase/ }),
    );
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();
    const startFresh = screen.getByRole("button", {
      name: /Start fresh/,
    }) as HTMLButtonElement;
    const importKp = screen.getByRole("button", {
      name: /Import keypair/,
    }) as HTMLButtonElement;
    expect(startFresh.disabled).toBe(true);
    expect(importKp.disabled).toBe(true);
  });
});

// ── Pure helpers ────────────────────────────────────────────────────────────

describe("Restore helpers", () => {
  it("truncatePubkey shows head/tail with ellipsis", () => {
    expect(truncatePubkey("H8xABCDEFGHIJKLMNOPQc4v")).toBe("H8xABC...Qc4v");
  });
  it("truncatePubkey leaves short pubkeys intact", () => {
    expect(truncatePubkey("Short")).toBe("Short");
  });
  it("formatCountdown handles seconds, minutes, hours, and zero", () => {
    expect(formatCountdown(0)).toBe("0s");
    expect(formatCountdown(-5)).toBe("0s");
    expect(formatCountdown(15_000)).toBe("15s");
    expect(formatCountdown(125_000)).toBe("2m 05s");
    expect(formatCountdown(3_700_000)).toMatch(/^1h /);
  });
});
