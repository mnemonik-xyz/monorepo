import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach } from "vitest";
import ShareDialog from "./ShareDialog";

vi.mock("../lib/wasm", () => ({
  loadWasm: vi.fn(async () => ({
    default: vi.fn(async () => undefined),
    unwrap_content_key: undefined, // not yet available
    wrap_content_key_to_recipient: undefined,
  })),
  __resetWasmForTests: vi.fn(),
}));

vi.mock("../lib/storage", () => ({
  readIdentity: vi.fn(() => ({
    secret: [1, 2, 3],
    pubkey_base58: "TestPubkey123",
  })),
}));

const FAKE_HASH =
  "a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4";
const FAKE_SEALED_BYTES = new Uint8Array([1, 2, 3, 4]);

function renderDialog(onClose = vi.fn()) {
  return render(
    <ShareDialog
      memoryHash={FAKE_HASH}
      sealedBytes={FAKE_SEALED_BYTES}
      onClose={onClose}
    />,
  );
}

describe("ShareDialog", () => {
  beforeEach(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => undefined) },
    });
  });

  it("renders_with_mode_buttons_and_continue_button", () => {
    renderDialog();
    expect(screen.getByTestId("share-dialog")).toBeInTheDocument();
    expect(screen.getByTestId("share-mode-did")).toBeInTheDocument();
    expect(screen.getByTestId("share-mode-link")).toBeInTheDocument();
    expect(screen.getByTestId("share-proceed")).toBeInTheDocument();
  });

  it("shows_recipient_input_in_did_mode", () => {
    renderDialog();
    expect(screen.getByTestId("share-recipient-input")).toBeInTheDocument();
  });

  it("proceed_button_disabled_in_did_mode_with_empty_recipient", () => {
    renderDialog();
    const btn = screen.getByTestId("share-proceed");
    expect(btn).toBeDisabled();
  });

  it("proceed_button_enabled_when_recipient_entered", async () => {
    const user = userEvent.setup();
    renderDialog();
    await user.type(
      screen.getByTestId("share-recipient-input"),
      "did:sol:Recipient123",
    );
    expect(screen.getByTestId("share-proceed")).not.toBeDisabled();
  });

  it("shows_no_revocation_warning_after_proceed_in_did_mode", async () => {
    const user = userEvent.setup();
    renderDialog();
    await user.type(
      screen.getByTestId("share-recipient-input"),
      "did:sol:Recipient123",
    );
    await user.click(screen.getByTestId("share-proceed"));
    // No-revocation warning must appear.
    expect(screen.getByTestId("share-warn")).toBeInTheDocument();
    expect(
      screen.getByText(/grants cannot be revoked/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/once the recipient has the key/i),
    ).toBeInTheDocument();
  });

  it("shows_no_revocation_warning_after_proceed_in_link_mode", async () => {
    const user = userEvent.setup();
    renderDialog();
    // Switch to link mode.
    await user.click(screen.getByTestId("share-mode-link"));
    await user.click(screen.getByTestId("share-proceed"));
    expect(screen.getByTestId("share-warn")).toBeInTheDocument();
    expect(
      screen.getByText(/bearer link is permanent read access/i),
    ).toBeInTheDocument();
  });

  it("cancel_from_warn_returns_to_choose_phase", async () => {
    const user = userEvent.setup();
    renderDialog();
    await user.type(
      screen.getByTestId("share-recipient-input"),
      "did:sol:Recipient123",
    );
    await user.click(screen.getByTestId("share-proceed"));
    expect(screen.getByTestId("share-warn")).toBeInTheDocument();

    await user.click(screen.getByTestId("share-cancel-warn"));
    // Should be back to the choose phase.
    expect(screen.queryByTestId("share-warn")).not.toBeInTheDocument();
    expect(screen.getByTestId("share-proceed")).toBeInTheDocument();
  });

  it("confirm_grant_shows_error_when_wasm_not_available", async () => {
    const user = userEvent.setup();
    renderDialog();
    await user.type(
      screen.getByTestId("share-recipient-input"),
      "did:sol:Recipient123",
    );
    await user.click(screen.getByTestId("share-proceed"));
    await user.click(screen.getByTestId("share-confirm"));
    // Should show an error since WASM sealed exports are not available.
    const error = await screen.findByTestId("share-error", {}, { timeout: 3000 });
    expect(error).toBeInTheDocument();
    expect(error.textContent).toMatch(/newer version|not available/i);
  });

  it("close_button_calls_onClose", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    renderDialog(onClose);
    await user.click(screen.getByRole("button", { name: /close share dialog/i }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
