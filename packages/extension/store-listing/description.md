# Mnemonik — Chrome Web Store listing copy

Paste-ready text for the developer dashboard. Two blocks:

1. **Short description** — 132 character hard limit, single line, no
   markdown. Goes in the tile under the title.
2. **Detailed description** — 16 000 character limit, plain text with
   blank-line paragraphs. Goes in the listing body.

Keep both blocks in sync with `manifest.json`'s `description` field
(which Chrome Web Store also shows in the install dialog).

---

## Short description (≤132 chars)

```
Capture context from any AI chat. Verifiable, portable memory across ChatGPT, Claude, Gemini and more.
```

(Character count: 102.)

---

## Detailed description (≤16 000 chars)

```
Mnemonik turns every AI chat you have into verifiable, portable memory.

Today your conversations with ChatGPT, Claude and Gemini live inside three different walled gardens. Switch tools and the context is gone. Audit something months later and you're trusting a vendor's database. Mnemonik fixes that by capturing the conversation locally, signing it with your own cryptographic identity, and storing the result either entirely on your device (free) or in managed cloud infrastructure (paid) — your choice, per device, switchable any time.

WHAT IT DOES

• Captures the full conversation from ChatGPT (chatgpt.com), Claude (claude.ai) and Gemini (gemini.google.com) with a single click on the popup or a keyboard shortcut. Per-platform adapters extract the turns in order and preserve role attribution.
• Right-click any selected text on any web page to save it as a memory. No "<all_urls>" permission required — activeTab plus a user gesture, full stop.
• Recall by semantic search: type a phrase, get the most relevant memories ranked by cosine similarity over a local embedding model. Copy as markdown, jump to the original page, or paste back into the current chat.
• Verify any memory at any time. Each is a COSE_Sign1 envelope over a deterministic CBOR-encoded payload, signed by your Ed25519 key. The popup's Verify tab re-runs the check on demand and flags tampering immediately.
• Cross-device under one identity. Bootstrap from your CLI or the webapp via a single-use ticket, or — if you want a recovery path — opt in to encrypted key escrow gated by a passphrase only you know.

LOCAL MODE (DEFAULT, FREE)

• Everything stays on this device. No sign-in. No network calls for the purpose of storing or syncing your memories.
• IndexedDB holds your memory index and the compressed embeddings.
• chrome.storage.local holds your Ed25519 keypair.
• No analytics, no telemetry, no SDK. The only outbound network request in Local mode is a one-time ~22 MB ONNX model download from Hugging Face on first capture.

CLOUD MODE (OPTIONAL, PAID)

• Sign in with Google in the popup. The extension uses chrome.identity.launchWebAuthFlow against our verified OAuth client, receives a signed id_token, and binds your account to that Google subject identifier (sub).
• Every signed attestation is sent over TLS 1.3 to mc.mnemonik.xyz and stored encrypted at rest on AWS (KMS-wrapped AES-256). You can list, recall, and delete from any logged-in device.
• Mnemonik can read the plaintext content of cloud attestations (no end-to-end encryption in this release — explicitly documented in our privacy policy). If that's not acceptable for your use case, stay in Local mode.
• Billed per-attestation outside the Chrome Web Store. Local mode never asks for payment.

KEY ESCROW (OPTIONAL, OFF BY DEFAULT)

• Want "sign in on a second laptop and recover everything"? Opt in to key escrow.
• Your passphrase never leaves the device. We derive a 256-bit key from it locally with Argon2id (OWASP-recommended parameters) and use it to wrap your Ed25519 secret with AES-GCM-256.
• The server stores only the wrapped ciphertext, keyed by your Google sub. We literally cannot decrypt it.
• Five fetches per 24 hours per account, rate-limited server-side.

PRIVACY

• Capture is always triggered by a user gesture — clicking the popup, the FAB, the context menu, or a keyboard shortcut. No background scraper, no silent observer.
• Auto-capture (where assistant turns are recorded as they arrive) is opt-in per domain and off by default.
• Telemetry is opt-in and anonymous. Off by default.
• host_permissions is enumerated (chatgpt.com, claude.ai, gemini.google.com). No "<all_urls>" anywhere.
• Full policy: https://mnemonik.xyz/extension/privacy

PRICING

• Local mode: free, forever. Everything on this device.
• Cloud mode: pay-as-you-go per attestation, settled outside the Chrome Web Store via x402 / Stripe. See mnemonik.xyz for current rates.

PERMISSIONS

• storage — persist your identity and settings.
• identity — Google sign-in flow (Cloud mode only).
• contextMenus — register the right-click "Save selection" item.
• activeTab — read the current tab on a user gesture (capture / save selection).
• clipboardWrite — copy a captured memory or a recall result as markdown.
• alarms — retry the cloud sync queue on a schedule.
• Host permissions: chatgpt.com, claude.ai, gemini.google.com — to run the per-platform conversation extractor.

OPEN SOURCE

• Apache-2.0 licensed. Full source: https://github.com/mnemonik-xyz/monorepo
• The CLI and the webapp share the same core (Rust + WASM). The extension's signing pipeline is byte-for-byte compatible: a memory captured in Chrome will verify in the CLI and vice versa.

SUPPORT

• Bugs: https://github.com/mnemonik-xyz/monorepo/issues
• Mail: hello@mnemonik.xyz
• Privacy: privacy@mnemonik.xyz
```

(Character count: ~3 980 — well inside the 16 000 cap.)

---

## Screenshot captions (5 × 1 280 × 800)

Used both in the listing carousel and as alt text. Keep each ≤ 80 chars.

| #   | File                                | Caption                                                          |
| --- | ----------------------------------- | ---------------------------------------------------------------- |
| 1   | `popup-capture.png`                 | One-click capture from ChatGPT, Claude or Gemini.                |
| 2   | `popup-recall.png`                  | Semantic recall over your local memories.                        |
| 3   | `fab-on-chatgpt.png`                | Floating action button on ChatGPT — capture without leaving the chat. |
| 4   | `options-storage-panel.png`         | Local / Cloud storage modes. Switch any time.                    |
| 5   | `restore-flow.png`                  | Restore your identity on a second device with one passphrase.    |

---

## Hero pitch (≤200 chars, used in social previews)

```
Verifiable memory for your AI conversations. Capture from ChatGPT, Claude and Gemini, search by meaning, prove it hasn't changed.
```

(Character count: 137.)

---

**Maintenance:** if `manifest.json` description or
`PRIVACY.md` § contact line changes, mirror it here before the next
store submission.
