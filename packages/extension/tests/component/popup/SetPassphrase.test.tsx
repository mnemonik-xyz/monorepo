// SetPassphrase-component tests (T17 round-2). The component wraps the
// freshly-minted Ed25519 secret, uploads the encrypted blob, and POSTs
// the possession-proof to /oauth/google/link. We bind:
//
//   1. zxcvbn < 3 keeps the submit button disabled.
//   2. length < 12 keeps the submit button disabled (covered by the
//      `isPassphraseAcceptable` helper — we observe the disabled state).
//   3. Passphrase confirmation mismatch keeps submit disabled and
//      renders the "does not match" warning.
//   4. Happy path: submit calls keyEscrow.wrap → keyEscrow.upload →
//      signChallenge → POST /oauth/google/link in sequence, then fires
//      onComplete.
//   5. wrap error renders a typed "Encrypt failed: …" message.
//   6. upload error renders a typed "Upload failed: …" message.
//   7. link 409 (already linked) renders "Link failed: already linked".
//
// The key-escrow facade is injected via the `keyEscrow` prop so we do
// not exercise WebCrypto in this layer (covered by the unit-test suite).
// The link POST is exercised through `fetchImpl` so we can observe the
// /oauth/google/link request body shape.

import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  SetPassphrase,
  type SetPassphraseKeyEscrow,
} from "../../../src/popup/onboarding/SetPassphrase.js";
import type { EscrowBlob } from "../../../src/auth/key-escrow.js";

// ── Fixtures ────────────────────────────────────────────────────────────────

const STRONG_PASSPHRASE = "correct horse battery staple long";
const SHORT_PASSPHRASE = "short-12c"; // length 9, < 12 floor
const WEAK_PASSPHRASE = "aaaaaaaaaaaa"; // length 12 but zxcvbn ~= 0

function makeKeypair(): { secret: Uint8Array; pubkey_base58: string } {
  const secret = new Uint8Array(64);
  for (let i = 0; i < 64; i++) secret[i] = i + 1;
  return { secret, pubkey_base58: "PubTestABC123" };
}

function makeBlob(): EscrowBlob {
  return {
    ciphertext_b64: "AAEC",
    nonce_b64: "AAECAwQFBgcICQoL",
    kdf: "argon2id",
    kdf_params: JSON.stringify({ salt_b64: "AAAA", m: 8, t: 1, p: 1 }),
    pubkey_base58: "PubTestABC123",
  };
}

function makeKeyEscrowSpy(): {
  ke: SetPassphraseKeyEscrow;
  wrapCalls: Array<{
    secret: Uint8Array;
    passphrase: string;
    pubkey: string;
  }>;
  uploadCalls: Array<{ jwt: string; blob: EscrowBlob }>;
} {
  const wrapCalls: Array<{
    secret: Uint8Array;
    passphrase: string;
    pubkey: string;
  }> = [];
  const uploadCalls: Array<{ jwt: string; blob: EscrowBlob }> = [];
  const ke: SetPassphraseKeyEscrow = {
    async wrap(secret, passphrase, pubkey) {
      wrapCalls.push({ secret, passphrase, pubkey });
      return makeBlob();
    },
    async upload(jwt, blob) {
      uploadCalls.push({ jwt, blob });
    },
  };
  return { ke, wrapCalls, uploadCalls };
}

function fetchOk(status = 200): typeof fetch {
  return (async () =>
    new Response(JSON.stringify({ status: "ok" }), {
      status,
      headers: { "content-type": "application/json" },
    })) as typeof fetch;
}

function fetchStatus(status: number, body: unknown = {}): typeof fetch {
  return (async () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    })) as typeof fetch;
}

// ── 1. Strength gate (zxcvbn < 3) ──────────────────────────────────────────

describe("SetPassphrase — passphrase strength gate", () => {
  it("submit disabled until zxcvbn >= 3 AND length >= 12 AND confirm matches", async () => {
    const { ke } = makeKeyEscrowSpy();
    render(
      <SetPassphrase
        jwt="JWT.PLACEHOLDER"
        keypair={makeKeypair()}
        linkChallenge={btoa("test-nonce")}
        signChallenge={async (n) => new Uint8Array(64).map((_, i) => i)}
        fetchImpl={fetchOk()}
        keyEscrow={ke}
        onComplete={() => undefined}
      />,
    );
    const pp1 = screen.getByLabelText(
      /Recovery passphrase/i,
    ) as HTMLInputElement;
    const pp2 = screen.getByLabelText(
      /Confirm passphrase/i,
    ) as HTMLInputElement;
    const submit = screen.getByRole("button", { name: /Encrypt & upload/i });
    expect((submit as HTMLButtonElement).disabled).toBe(true);

    // Length OK but weak — should still be disabled.
    fireEvent.change(pp1, { target: { value: WEAK_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: WEAK_PASSPHRASE } });
    expect((submit as HTMLButtonElement).disabled).toBe(true);

    // Too short — disabled.
    fireEvent.change(pp1, { target: { value: SHORT_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: SHORT_PASSPHRASE } });
    expect((submit as HTMLButtonElement).disabled).toBe(true);

    // Strong + matched — enabled.
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: STRONG_PASSPHRASE } });
    await waitFor(() => {
      expect((submit as HTMLButtonElement).disabled).toBe(false);
    });
  });

  it("renders 'does not match' when confirm differs from passphrase", async () => {
    const { ke } = makeKeyEscrowSpy();
    render(
      <SetPassphrase
        jwt="JWT.PLACEHOLDER"
        keypair={makeKeypair()}
        linkChallenge={btoa("test-nonce")}
        signChallenge={async () => new Uint8Array(64)}
        fetchImpl={fetchOk()}
        keyEscrow={ke}
        onComplete={() => undefined}
      />,
    );
    const pp1 = screen.getByLabelText(
      /Recovery passphrase/i,
    ) as HTMLInputElement;
    const pp2 = screen.getByLabelText(
      /Confirm passphrase/i,
    ) as HTMLInputElement;
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, {
      target: { value: "different-passphrase-12-chars" },
    });
    await waitFor(() => {
      expect(screen.getByText(/Confirmation does not match/i)).toBeTruthy();
    });
    const submit = screen.getByRole("button", { name: /Encrypt & upload/i });
    expect((submit as HTMLButtonElement).disabled).toBe(true);
  });
});

// ── 4. Happy path — wrap → upload → signChallenge → link → onComplete ─────

describe("SetPassphrase — happy path", () => {
  it("submits the full wrap → upload → link sequence in order", async () => {
    const keypair = makeKeypair();
    const { ke, wrapCalls, uploadCalls } = makeKeyEscrowSpy();
    const signCalls: Uint8Array[] = [];
    const signChallenge = async (n: Uint8Array): Promise<Uint8Array> => {
      signCalls.push(n);
      const sig = new Uint8Array(64);
      for (let i = 0; i < 64; i++) sig[i] = (i + 7) & 0xff;
      return sig;
    };
    const fetchCalls: Array<{ url: string; init: RequestInit }> = [];
    const fetchImpl = (async (input: RequestInfo | URL, init?: RequestInit) => {
      fetchCalls.push({
        url: typeof input === "string" ? input : input.toString(),
        init: init ?? {},
      });
      return new Response(JSON.stringify({ status: "ok" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;
    let completed = false;
    render(
      <SetPassphrase
        jwt="JWT.PLACEHOLDER"
        keypair={keypair}
        linkChallenge={btoa("happy-nonce")}
        signChallenge={signChallenge}
        fetchImpl={fetchImpl}
        keyEscrow={ke}
        onComplete={() => {
          completed = true;
        }}
      />,
    );
    const pp1 = screen.getByLabelText(/Recovery passphrase/i);
    const pp2 = screen.getByLabelText(/Confirm passphrase/i);
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: STRONG_PASSPHRASE } });
    await waitFor(() => {
      expect(
        (
          screen.getByRole("button", {
            name: /Encrypt & upload/i,
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false);
    });
    fireEvent.click(screen.getByRole("button", { name: /Encrypt & upload/i }));
    await waitFor(() => {
      expect(completed).toBe(true);
    });
    // 1. wrap got the keypair's secret + STRONG_PASSPHRASE + pubkey.
    expect(wrapCalls.length).toBe(1);
    expect(wrapCalls[0]!.secret).toEqual(keypair.secret);
    expect(wrapCalls[0]!.passphrase).toBe(STRONG_PASSPHRASE);
    expect(wrapCalls[0]!.pubkey).toBe(keypair.pubkey_base58);
    // 2. upload was called with the produced blob.
    expect(uploadCalls.length).toBe(1);
    expect(uploadCalls[0]!.jwt).toBe("JWT.PLACEHOLDER");
    expect(uploadCalls[0]!.blob.pubkey_base58).toBe(keypair.pubkey_base58);
    // 3. signChallenge called with the decoded nonce bytes.
    expect(signCalls.length).toBe(1);
    const expectedNonce = new TextEncoder().encode("happy-nonce");
    // Compare byte-for-byte without depending on Uint8Array identity —
    // jsdom + vitest sometimes produce Uint8Array instances whose
    // constructors differ across realms.
    expect(Array.from(signCalls[0]!)).toEqual(Array.from(expectedNonce));
    // 4. /oauth/google/link POST carries pubkey + base64 signature + challenge.
    expect(fetchCalls.length).toBe(1);
    const linkCall = fetchCalls[0]!;
    expect(linkCall.url).toMatch(/\/oauth\/google\/link$/);
    expect(linkCall.init.method).toBe("POST");
    const body = JSON.parse(linkCall.init.body as string) as Record<
      string,
      unknown
    >;
    expect(body.pubkey_base58).toBe(keypair.pubkey_base58);
    expect(typeof body.possession_proof_base64).toBe("string");
    expect((body.possession_proof_base64 as string).length).toBeGreaterThan(0);
    expect(body.challenge).toBe(btoa("happy-nonce"));
  });
});

// ── 5. wrap error renders typed "Encrypt failed" copy ─────────────────────

describe("SetPassphrase — wrap error", () => {
  it("renders 'Encrypt failed: …' and stays on the form", async () => {
    const ke: SetPassphraseKeyEscrow = {
      async wrap() {
        throw new Error("argon2 crashed");
      },
      async upload() {
        return;
      },
    };
    render(
      <SetPassphrase
        jwt="JWT.PLACEHOLDER"
        keypair={makeKeypair()}
        linkChallenge={btoa("nonce")}
        signChallenge={async () => new Uint8Array(64)}
        fetchImpl={fetchOk()}
        keyEscrow={ke}
        onComplete={() => undefined}
      />,
    );
    const pp1 = screen.getByLabelText(/Recovery passphrase/i);
    const pp2 = screen.getByLabelText(/Confirm passphrase/i);
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: STRONG_PASSPHRASE } });
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: /Encrypt & upload/i,
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: /Encrypt & upload/i }));
    await waitFor(() => {
      expect(screen.getByText(/Encrypt failed: argon2 crashed/i)).toBeTruthy();
    });
  });
});

// ── 6. upload error renders typed "Upload failed" copy ────────────────────

describe("SetPassphrase — upload error", () => {
  it("renders 'Upload failed: …' on uploadEscrow rejection", async () => {
    const ke: SetPassphraseKeyEscrow = {
      async wrap() {
        return makeBlob();
      },
      async upload() {
        throw new Error("403 forbidden");
      },
    };
    render(
      <SetPassphrase
        jwt="JWT.PLACEHOLDER"
        keypair={makeKeypair()}
        linkChallenge={btoa("nonce")}
        signChallenge={async () => new Uint8Array(64)}
        fetchImpl={fetchOk()}
        keyEscrow={ke}
        onComplete={() => undefined}
      />,
    );
    const pp1 = screen.getByLabelText(/Recovery passphrase/i);
    const pp2 = screen.getByLabelText(/Confirm passphrase/i);
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: STRONG_PASSPHRASE } });
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: /Encrypt & upload/i,
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: /Encrypt & upload/i }));
    await waitFor(() => {
      expect(screen.getByText(/Upload failed: 403 forbidden/i)).toBeTruthy();
    });
  });
});

// ── 7. /oauth/google/link 409 renders typed "already linked" copy ─────────

describe("SetPassphrase — link 409 (already linked)", () => {
  it("renders 'Link failed: already linked' when /oauth/google/link returns 409", async () => {
    const { ke } = makeKeyEscrowSpy();
    let completed = false;
    render(
      <SetPassphrase
        jwt="JWT.PLACEHOLDER"
        keypair={makeKeypair()}
        linkChallenge={btoa("nonce")}
        signChallenge={async () => {
          const sig = new Uint8Array(64);
          sig.fill(7);
          return sig;
        }}
        fetchImpl={fetchStatus(409, { error: "already_linked" })}
        keyEscrow={ke}
        onComplete={() => {
          completed = true;
        }}
      />,
    );
    const pp1 = screen.getByLabelText(/Recovery passphrase/i);
    const pp2 = screen.getByLabelText(/Confirm passphrase/i);
    fireEvent.change(pp1, { target: { value: STRONG_PASSPHRASE } });
    fireEvent.change(pp2, { target: { value: STRONG_PASSPHRASE } });
    await waitFor(() =>
      expect(
        (
          screen.getByRole("button", {
            name: /Encrypt & upload/i,
          }) as HTMLButtonElement
        ).disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: /Encrypt & upload/i }));
    await waitFor(() => {
      expect(screen.getByText(/Link failed.*already linked/i)).toBeTruthy();
    });
    expect(completed).toBe(false);
  });
});
