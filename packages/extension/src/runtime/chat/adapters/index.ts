// Adapter bootstrap. Importing this module registers every concrete
// `ChatAdapter` with the registry as an import side-effect. T07 wires
// ChatGPT; T08/T09 will add `./claude.adapter.js` and
// `./gemini.adapter.js` lines below — keep registrations alphabetical
// so concurrent feature branches edit different lines and merge cleanly.

import "./chatgpt.adapter.js";
