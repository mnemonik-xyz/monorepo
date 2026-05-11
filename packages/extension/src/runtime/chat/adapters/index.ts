// Adapter bootstrap. Importing this module registers every concrete
// `ChatAdapter` with the registry as an import side-effect. Each
// adapter self-registers under its supported origin; the popup-realm
// `selectAdapter(url)` then resolves the right one per active tab.
// Keep registrations alphabetical so concurrent feature branches edit
// different lines and merge cleanly.

import "./chatgpt.adapter.js";
import "./claude.adapter.js";
import "./gemini.adapter.js";
