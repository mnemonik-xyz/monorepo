// Adapter bootstrap. Importing this module registers every concrete
// `ChatAdapter` with the registry as an import side-effect. Keep
// registrations alphabetical so concurrent feature branches edit
// different lines and merge cleanly.
//
// Audit M1 / AUD-C-06: T07 shipped ChatGPT; T08 + T09 shipped Claude +
// Gemini but their self-registering modules were never imported from
// this central index. The popup is a separate JS realm from the
// content script (manifest content_scripts wires each adapter per
// origin into the content-script realm only), so the popup-side
// `selectAdapter` was returning null on claude.ai / gemini.google.com
// pages and the popup's "Save chat" button stayed disabled.

import "./chatgpt.adapter.js";
import "./claude.adapter.js";
import "./gemini.adapter.js";
