# Mnemonik Chrome Extension — Privacy Policy

**Effective date:** 2026-05-11
**Version:** 1.0.0
**Contact:** privacy@mnemonik.xyz
**Hosted copy:** https://mnemonik.xyz/extension/privacy

This document explains exactly what data the Mnemonik Chrome extension
("the extension") collects, where it goes, who can read it, and how you
can delete it. It is written to match the actual code that ships in
Chrome Web Store version 1.0.0 — not aspirational. If the code and this
policy disagree, the code is the bug.

## 1. TL;DR

- **Local mode (default, free):** everything stays on your device. The
  extension makes no network requests for the purpose of storing,
  syncing, or analysing your memories. No analytics. No telemetry.
- **Cloud mode (optional, paid):** signed attestations are sent over
  TLS to `mc.mnemonik.xyz`, stored encrypted at rest on AWS, and linked
  to your Google account by the opaque Google subject identifier
  (`sub`). The Mnemonik server can read the plaintext content of cloud
  attestations — there is no end-to-end encryption in this release
  (Decision D8). If you do not want the server to be able to read your
  content, stay in Local mode.
- **Key escrow (optional):** if you enable Google-login restore, the
  server stores a *passphrase-encrypted* blob of your private key. It
  cannot decrypt the blob. The passphrase never leaves your device.
- **Telemetry:** opt-in only, anonymous, disabled by default.
- **Right to deletion:** one click removes your local data; one HTTP
  call (`DELETE /api/key-escrow`) removes your server-side escrow; on
  written request to `privacy@mnemonik.xyz` we delete all
  server-stored attestations linked to your account within 30 days.
  A self-service `DELETE /api/memories` endpoint is on the roadmap;
  until it ships, attestation deletion is handled via the email
  request above (GDPR Art. 17 — same 30-day SLA either way).

## 2. Two storage modes — pick one per device

The extension supports two mutually exclusive storage modes
(Decision D7 — explicit mode switching, no silent dual-write):

### 2.1 Local mode

- **Default on first install.** No sign-in. No network. No server.
- All memories live in IndexedDB on the device that captured them.
- The Ed25519 identity used to sign attestations is generated locally
  and stored in `chrome.storage.local`. The private key never leaves
  the device unless *you* explicitly export it (file download) or
  enable key escrow (§4 below). Per our design (Decision D6) the
  same Ed25519 keypair represents your identity across the CLI,
  webapp, and extension — there is one identity per person, not one
  per client — so keep it safe.
- No analytics SDK, no error-reporting beacon, no remote telemetry.
- The only outbound network requests in Local mode are the
  first-run model fetches from `huggingface.co` /
  `cdn-lfs.huggingface.co` (~25 MB ONNX embedding model — required to
  compute the semantic vectors stored alongside each memory). These
  are static asset downloads; the request body contains no personal
  data. The CSP allow-lists exactly these origins (`connect-src` in
  `manifest.json`).

### 2.2 Cloud mode (paid)

You opt in explicitly from the Options page. Switching to Cloud mode
requires:

1. Sign in with Google (`chrome.identity.launchWebAuthFlow`). We
   receive the `id_token` JWT issued by Google. From it we read the
   `sub` (opaque subject id) and use that as the row key on our side.
   We do not store the JWT itself beyond the lifetime of one request.
2. Bind an X-Payment authorisation header so your account is
   metered for paid attestation storage.

Once in Cloud mode, every `sign_memory` call:

1. Builds and signs the attestation locally (your Ed25519 key — same
   as Local mode).
2. Sends the COSE_Sign1 envelope over TLS 1.3 to `mc.mnemonik.xyz`.
3. The server stores the envelope encrypted at rest (AWS KMS-wrapped
   AES-256 on S3 + RDS) and indexed by your Google `sub`.

**The server can read the plaintext attestation content** once it
decrypts at-rest. This is Decision D8 — end-to-end encryption is
deferred to a future release. If this is unacceptable for your use
case, stay in Local mode.

We do not sell, rent, or share attestation content with any third
party. We use it only to fulfil `recall` requests you make against
your own account.

## 3. What we collect, by category

### 3.1 Account identifiers (Cloud mode only)

| Field                | Source                       | Purpose                       | Retention                 |
| -------------------- | ---------------------------- | ----------------------------- | ------------------------- |
| Google `sub`         | Google id_token              | Row key on the Mnemonik API   | Until you request deletion |
| `email`, `name`      | Google id_token              | Displayed in Options UI only; not used as a join key | Same |
| `pubkey_base58`      | Your local Ed25519 keypair   | Verify signatures server-side | Same |

We do **not** receive your Google password or refresh token. We do not
read your Gmail, Drive, Calendar, or any other Google service.

### 3.2 Attestation content (Cloud mode only)

When you click "Capture" in the popup or use the right-click "Save
selection" context menu, the extension sends to the server:

- The captured text (the user-selected text, or — for AI chats — the
  ordered conversation turns extracted by the per-domain adapter).
- A small set of provenance fields: page URL, page title, the
  adapter id ("chatgpt" / "claude" / "gemini" / "generic"), the
  conversation id when the adapter can extract one, and the
  timestamp at the moment of capture.
- The Ed25519 signature you produced locally.
- The compressed semantic embedding (a few hundred bytes).

We never silently capture anything. No background timer scrapes the
page. Capture is always triggered by a user gesture (popup button,
context-menu item, or keyboard shortcut). Auto-capture is opt-in per
domain and disabled by default (Decision D12).

### 3.3 Embeddings

Embeddings are computed locally using the `Xenova/all-MiniLM-L6-v2`
ONNX model (384-dimensional sentence embeddings; see Decision D3).
The model file itself is downloaded once from `huggingface.co` on
first use; thereafter it is served from the in-extension cache. The
text you embed never reaches Hugging Face's servers.

### 3.4 Diagnostic / crash data

Off by default. The extension does not ship a crash reporter, an
analytics SDK, a session-replay tool, or a feature-flag service. If
you enable the opt-in telemetry toggle in Options (off by default,
described in §6), we send only anonymous counters — no content, no
identifiers, no IP retention beyond the standard TLS request log.

### 3.5 What we do NOT collect

- Browsing history outside the three adapter domains.
- Tabs you have open, page contents you have not explicitly captured.
- Form input, keystrokes, mouse movement, scroll depth.
- Device fingerprinting beyond what is necessary for TLS.
- Cross-site identifiers, advertising ids, third-party cookies.

## 4. Key escrow (optional, off by default)

To support "sign in with Google on a second device and recover your
identity," we offer optional key escrow. It is **off by default** and
gated by an explicit opt-in in the popup onboarding flow and the
Options → Security panel.

When you enable it:

1. You enter a recovery passphrase that *never leaves your device*.
2. The extension runs Argon2id locally with parameters
   `memory_cost=64 MiB, time_cost=3, parallelism=1` (OWASP 2023+)
   over your passphrase + a random 16-byte salt.
3. The derived 256-bit key wraps your Ed25519 secret with AES-GCM-256
   (random 12-byte nonce per wrap).
4. The extension uploads the *ciphertext blob*, the salt, the nonce,
   the KDF parameters, and your public key. **No plaintext key
   material, no passphrase, no KDF output — ever — leaves the
   device.**
5. The server stores this opaque blob keyed by your Google `sub`.
   It cannot decrypt the blob; without your passphrase it sees
   bytes.

Server-side rate limit: 5 GET fetches per 24 h per Google `sub`. This
bounds online brute force. Argon2id cost bounds offline brute force
if the blob is ever stolen.

If you lose your passphrase, you lose the ability to restore from
escrow — fall back to the manual export-keypair / import-keypair
flow from another logged-in device. This is intentional (Decision
D9 — server-zero-knowledge escrow).

You can revoke an escrowed key at any time:

```
DELETE /api/key-escrow
Authorization: Bearer <google_id_token>
```

The Options → Security → "Forget escrow on server" button issues
this request for you.

## 5. Where the data lives (sub-processors)

| Sub-processor              | Purpose                                    | Region(s)            |
| -------------------------- | ------------------------------------------ | -------------------- |
| Amazon Web Services        | S3 (attestation blobs), RDS (metadata)     | eu-central-1 primary |
| Google LLC                 | OAuth provider for sign-in (Cloud mode)    | Global               |
| Hugging Face, Inc.         | One-time static model download             | Global CDN           |
| Cloudflare, Inc.           | TLS termination + edge caching for static  | Global               |

We do not use any advertising, analytics, or marketing sub-processors
in the extension.

## 6. Telemetry — opt-in, anonymous, off by default

The Options → Privacy section contains a single toggle: **"Send
anonymous usage counters."** It is disabled out of the box.

When (and only when) you enable it, the extension reports to
`mc.mnemonik.xyz/telemetry`:

- counter: number of captures, recalls, and verifies in the last 24 h
- counter: error code histogram (no message bodies, no stack traces)
- string: extension version
- string: a per-install random UUID generated locally (resettable
  any time from the same panel)

Specifically *not* reported: the captured text, embeddings, page URLs,
adapter ids, identity public keys, Google account fields, the
recovery passphrase, IP addresses beyond the standard request log.

## 7. Permissions — why we ask, what we do with each

The extension declares only the permissions it actually needs.
There is no `<all_urls>` host_permission anywhere in `manifest.json`
(Decision D11). The list:

| Permission         | Why                                                                    |
| ------------------ | ---------------------------------------------------------------------- |
| `storage`          | Persist your identity, settings, and the local memory index.           |
| `identity`         | Run `chrome.identity.launchWebAuthFlow` for Google sign-in.            |
| `contextMenus`     | Register the right-click "Save selection" item.                        |
| `activeTab`        | Read the current tab's selection / DOM *only* when you act.            |
| `clipboardWrite`   | Copy a captured memory or recall result as markdown.                   |
| `alarms`           | Periodically retry the cloud-sync queue when on Wi-Fi.                 |
| `https://chatgpt.com/*` | Run the ChatGPT adapter on that origin.                           |
| `https://claude.ai/*`   | Run the Claude.ai adapter on that origin.                         |
| `https://gemini.google.com/*` | Run the Gemini adapter on that origin.                      |

`host_permissions` is enumerated per supported chat domain rather
than `<all_urls>` so that the extension only ever sees pages where
it has been explicitly authorised to run.

The extension's Content Security Policy allows `wasm-unsafe-eval`
(and only `wasm-unsafe-eval` — never `unsafe-eval`) so it can load
the on-device embedder and signing WASM module. This is the standard
Manifest V3 directive for WebAssembly and does not permit JavaScript
`eval()`, dynamic `new Function(...)`, or any remote-code execution.

## 8. Your rights — access, correction, deletion, portability

You can do all of the following without contacting us:

- **Access:** the Options page shows every memory the extension knows
  about and lets you export the full local store as a JSON file.
- **Correction:** delete and re-capture; attestation content is
  immutable by design (that is what "verifiable" means).
- **Deletion (local):** Options → Storage → "Clear local store"
  wipes IndexedDB + `chrome.storage.local`.
- **Deletion (cloud — escrow):** Options → Cloud → "Forget escrow
  on server" triggers `DELETE /api/key-escrow`. Server-side hard
  delete completes within seconds.
- **Deletion (cloud — attestations):** email `privacy@mnemonik.xyz`
  from the address bound to your Google account; we delete all
  server-stored attestations linked to your `sub` within 30 days
  (GDPR Art. 17). A self-service `DELETE /api/memories` endpoint is
  on the roadmap and will replace the email step when it ships.
- **Portability:** the export JSON above is the same format the CLI
  consumes — `mnemonic identity import --file ./export.json`.

If anything above fails, email `privacy@mnemonik.xyz` and we will
complete the deletion within 30 days. EEA / UK residents may also
contact the data-protection officer at the same address.

## 9. Children

The extension is not directed at children under 13. We do not
knowingly collect personal data from children.

## 10. Changes to this policy

Substantive changes (e.g. a new sub-processor, a new data category)
will be announced on https://mnemonik.xyz/extension/privacy and in
the Chrome Web Store changelog at least 14 days before they take
effect. Minor wording fixes get a quiet bump to the `Effective date`
above.

## 11. Contact

- Privacy & DPA: `privacy@mnemonik.xyz`
- General support: `hello@mnemonik.xyz`
- Source code: https://github.com/mnemonik-xyz/monorepo

Data controller: Mnemonik, the project organisation behind the
above repository. A written DPA is available on request from
`privacy@mnemonik.xyz`.

