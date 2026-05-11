# Chrome Web Store — Permissions justifications

Paste each block into the matching field in *Privacy practices →
Permission justifications* on the developer dashboard. One paragraph
per permission; Chrome Web Store reviewers cross-check these against
the manifest line-by-line.

The manifest declares no `<all_urls>` host_permission anywhere
(Decision D11). Adding a fourth chat platform would be a versioned
release with the new origin enumerated explicitly.

---

## `storage`

We use `chrome.storage.local` to persist the user's Ed25519 identity
keypair, the per-domain auto-capture opt-in flags, the user's choice
of storage mode (Local vs Cloud), and the cloud-sync retry queue
metadata. We use IndexedDB (also under the `storage` capability) for
the local memory index and the compressed embeddings. Without
`storage` the extension would forget the user's identity on every
service-worker shutdown and could not back its semantic-search
feature.

## `identity`

Required only for Cloud mode. We call
`chrome.identity.launchWebAuthFlow` against our verified Google
OAuth client to obtain a signed `id_token`. The token's `sub` (opaque
subject id) becomes the row key on the Mnemonik API. We never
receive the user's Google password, refresh token, or any service-
specific scope beyond `openid email profile`. Users on Local mode
never hit this code path.

## `contextMenus`

We register a single right-click item, "Save selection to Mnemonik",
which is the user-gesture surface that triggers a `save-selection`
capture on the active tab without requiring a broad host_permission.
The item only appears when the user has actually selected text.
Without `contextMenus` we would have to nag the user to open the
popup for every off-platform capture.

## `activeTab`

Used to read the current tab's selection or DOM *only* when the user
takes an explicit action (popup button, context-menu item, keyboard
shortcut). `activeTab` is granted by Chrome for one tab at a time on
that user gesture and is revoked on tab change. This is the
mechanism that lets us support generic page capture without a
`<all_urls>` host_permission.

## `alarms`

The service worker uses `chrome.alarms` to retry the cloud-sync
queue on a 60-second cadence while items remain in the failed-to-
upload state (network drop, transient 5xx). The alarm fires only
when items exist in the queue and stops automatically when the
queue drains. Without `alarms` we would have to keep the service
worker alive with a background timer, which MV3 explicitly forbids.

## `host_permissions: https://chatgpt.com/*`

The per-domain content script runs the ChatGPT conversation
extractor: it watches the assistant's response DOM with a settled-
MutationObserver, parses the ordered turns, and surfaces them to the
popup on a user gesture. The scope is exactly the chat origin; the
adapter does nothing on chatgpt.com pages other than wait for an
explicit capture trigger.

## `host_permissions: https://claude.ai/*`

Same as the ChatGPT entry, scoped to Anthropic's Claude.ai chat
origin. Runs the Claude-specific adapter (different DOM, different
turn structure). Idle until the user clicks Capture / context-menu /
hotkey.

## `host_permissions: https://gemini.google.com/*`

Same as above, scoped to Google's Gemini chat origin. Runs the
Gemini-specific adapter. Idle until the user clicks Capture / context-
menu / hotkey. We do not touch any other Google domain.

---

## CSP justification

The extension ships a strict MV3 CSP:

```
script-src 'self' 'wasm-unsafe-eval';
worker-src 'self';
object-src 'self';
base-uri 'self';
connect-src 'self'
  https://mc.mnemonik.xyz
  https://huggingface.co
  https://cdn-lfs.huggingface.co
  https://cdn-lfs-us-1.huggingface.co
```

- `'wasm-unsafe-eval'` is required because the bundle ships a WASM
  module (the signing pipeline shared with the CLI). Chrome's MV3
  spec explicitly allows this directive for WASM with no
  remote-code-execution risk.
- `'unsafe-eval'` is **not** present.
- `connect-src` is allow-listed to the Mnemonik API origin plus the
  Hugging Face CDN origins from which the one-time embedding-model
  download is served. No other outbound origins.
