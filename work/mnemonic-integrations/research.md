---
created: 2026-04-26
status: research
type: feature-research
size: M
---

# Research: Shipping Attested Context Bundles into Mainstream AI Apps

> Source: research subagent dispatched 2026-04-26 against the question
> "How to deliver compressed RAG / attested context bundles directly into a
> fresh chat in another AI app, without forcing users to download a file?"
> This document is the seed artifact for the `mnemonic-integrations` feature.

## 1. TL;DR / Recommended Path

**Ship a hosted, OAuth-protected remote MCP server as your single primary
integration, paired with a "Copy as prompt" clipboard fallback and a
per-bundle short-link landing page.** As of April 2026, MCP has won as the
de facto context-handoff standard: Claude.ai (Pro/Max/Team/Enterprise),
Claude Desktop, Cursor, VS Code/Copilot, Windsurf, Zed, and Perplexity
(Pro/Max/Enterprise) all ingest custom remote MCP servers via a URL +
OAuth flow, and ChatGPT now does too in Developer Mode (Business /
Enterprise / Edu, with Plus/Pro able to use admin-published connectors).
One MCP endpoint covers ~70% of your reachable users with verifiable,
signature-preserving handoff. The rest get a clipboard handoff
(universal, two clicks, signature-lossy) and a permalink landing page
(`mnemonic.dev/b/<id>`) with one-click install deeplinks for Cursor /
VS Code / Claude Desktop and copy-as-prompt buttons for ChatGPT /
Claude.ai / Perplexity / Gemini. **Defer** the ChatGPT App Store
submission and a browser extension to phase 2 — both are real options
but slow (review time) or risky (permissions, ShadowPrompt-style attack
surface).

## 2. Surface-by-Surface Matrix

| Destination | Best mechanism (today) | Friction (clicks) | Dev effort 1–5 | Signature preserved? | Reach |
|---|---|---|---|---|---|
| **ChatGPT web (Plus/Pro)** | "Copy as prompt" + open `chatgpt.com/?q=` | 2 (copy → paste) | 1 | No (paste loses signature) | High — but message length cap ~6k chars |
| **ChatGPT web (Business/Enterprise/Edu)** | Custom MCP connector via Developer Mode | 3–4 (paste URL, OAuth, enable in chat) | 3 (server already exists) | Yes (verifiable on tool call) | Medium reach, high value users |
| **ChatGPT App Store (all tiers)** | Submit Apps SDK app (MCP server + UI) | 1 once approved | 4 (submission, review, demo account, privacy policy) | Yes | Highest reach if approved; review time uncertain |
| **ChatGPT Desktop / Atlas browser** | Same as ChatGPT web; MCP connector inherits | Same | Same | Yes | Same |
| **Claude.ai web (Pro/Max/Team/Ent)** | Custom remote MCP connector (Settings → Connectors → Add custom connector → URL) | 3 (paste URL + OAuth) | 3 | Yes | Highest among Claude users |
| **Claude.ai web (Free)** | One custom connector slot OR clipboard handoff | 3 or 2 | 1–3 | Yes / No | Limited (1 connector) |
| **Claude Desktop** | `.mcpb` desktop extension (one-click install) OR remote MCP URL | 1–2 (double-click `.mcpb`) | 2–3 | Yes | High |
| **Cursor** | `cursor://anysphere.cursor-deeplink/mcp/install?...` one-click button | 1 | 1 (just URL-encode config) | Yes | High among devs |
| **VS Code / Copilot** | `vscode:mcp/install` deeplink or `@mcp` marketplace entry | 1 | 1–2 | Yes | High among devs |
| **Windsurf / Zed** | Remote MCP URL paste; no standardized deeplink yet | 2–3 | 1 | Yes | Medium |
| **Perplexity (Pro/Max/Enterprise)** | Custom remote MCP connector; OAuth/API key/none | 3 | 1 (reuse server) | Yes | Medium |
| **Gemini (consumer Gems)** | No public connector path; clipboard / Google Drive file sync | 3+ | 1 | No | Medium-low |
| **Gemini Enterprise** | Custom data connector (Fetch/Transform/Sync pipeline) | n/a (admin-only) | 5 | Partial (data-store level) | Niche |
| **Gemini CLI** | Gemini CLI extension (MCP-based) | 1 | 2 | Yes | Devs only |
| **Le Chat / others** | Clipboard handoff or short-link | 2 | 1 | No | Long tail |

## 3. Mechanism Deep-Dives

### 3.1 Hosted Remote MCP Server (your primary lever)

**Pros:** One artifact (an HTTP MCP endpoint behind OAuth) is consumed by
Claude.ai web, Claude Desktop, Claude Code Web, Cursor, VS Code,
Windsurf, Zed, Perplexity, and ChatGPT (Developer Mode + Apps SDK). Tool
calls preserve cryptographic verification — your `verify` tool can
return the COSE-signed CBOR bundle and the client model can be
instructed to acknowledge the Solana memo + Ed25519 chain. This is *the
only* mechanism that universally preserves the attestation.

**Cons:** Still asks the user to paste a URL and complete OAuth. ChatGPT
Plus/Pro cannot install custom MCP themselves (only Business/Enterprise/
Edu do, or via published Apps SDK app). MCP servers must be reachable
from Anthropic/OpenAI cloud IP ranges (no VPN, no localhost).

**To ship:**
1. Migrate `mnemonic-mcp` from stdio to streamable HTTP (the spec OpenAI
   and Anthropic both require — SSE is being phased out). The MCP
   Rust/TypeScript SDKs both support this.
2. Add OAuth 2.1 with PKCE; map tokens to user identities. Anthropic's
   docs explicitly require public-internet reachability and OAuth for
   paid-tier connectors.
3. Expose existing 5 tools (`whoami`, `sign_memory`, `verify`,
   `prove_identity`, `recall`) but add a thin `get_protocol_knowledge`
   tool that returns the attested bundle for the marketing-site visitor
   flow.
4. Test in Cursor (`cursor://anysphere.cursor-deeplink/mcp/install?name=Mnemonic&config=<base64>`)
   and Claude.ai (Settings → Connectors → Add custom connector).

### 3.2 OAuth + First-Party Connector Stores

**ChatGPT Apps SDK / App Directory:** Open to verified developers since
late 2025; live for early-2026 rollouts. Submission requires: a working
MCP server, a demo login, privacy policy, no digital-goods commerce, and
review (no published SLA — community reports days-to-weeks). Once
approved, an app shows up in ChatGPT's in-chat directory across Free/
Plus/Pro/Business/Enterprise — this is the only path to ChatGPT Plus/Pro
users without asking them to paste config. **Risk:** Your "verify-an-
Arweave-bundle-of-a-Solana-anchored-attestation" tool surface is
unusual; review feedback may push back on the crypto/blockchain framing.
Position the app as "verifiable knowledge memory," lead with utility,
treat the chain as plumbing.

**Claude Connectors Directory:** Anthropic's pre-built connector
directory has 50+ integrations (Gmail, Slack, Notion, Figma, etc.) as of
February 2026. Custom remote MCP works without listing. Pre-built
listing is partner-led; no public submission portal that I can confirm.
Claude users can self-add any custom MCP URL — listing is purely about
discovery, so it's optional for phase 1.

**Pros:** Highest discoverability; your protocol shows up where users
already look. **Cons:** Slow review, third-party gatekeeping, some loss
of control over how it's framed.

### 3.3 `.mcpb` Desktop Extension (Claude Desktop)

A `.mcpb` is a zip with a `manifest.json` + the local MCP server, like
a `.crx`/`.vsix`. Users double-click to install in Claude Desktop,
Claude Code, MCP-for-Windows. One-click is *real* (not "configure the
JSON yourself"). **But:** It runs the server *locally on the user's
machine*, which only makes sense if you ship a local Rust binary. Given
`mnemonic-mcp` already runs as stdio, this is cheap. Useful as a
fallback for users who want offline/local operation, but not the
primary path — your bundles live on Arweave, so a remote MCP is more
honest about the architecture.

### 3.4 IDE Deeplinks (Cursor, VS Code, Windsurf)

Cursor: `cursor://anysphere.cursor-deeplink/mcp/install?name=Mnemonic&config=<base64-json-of-{"url":"https://mcp.mnemonic.dev"}>`.
One click, deeplink launches Cursor and prompts for install. VS Code has
`vscode:mcp/install` and an `@mcp` marketplace search. These are the
lowest-friction installs for devs and almost free to ship — just
generate the URL and stick a button on your landing page. **Caveat:**
Proofpoint published a "CursorJack" advisory about deeplink phishing;
users should verify the origin. Don't host the install link from a URL
shortener; serve from your verified domain over HTTPS.

### 3.5 Browser Extension

Inject the bundle as a hidden first user message or pre-fill the input
on `chatgpt.com`, `claude.ai`, `perplexity.ai`. **Pros:** Universal
across destinations and plan tiers, including ChatGPT Free/Plus where
MCP is gated. **Cons:**

- **Big maintenance/security burden.** ShadowPrompt (March 2026) was a
  zero-click prompt injection in Anthropic's *own* Claude Chrome
  extension via a `*.claude.ai` allowlist + an XSS in an Arkose Labs
  subdomain. If Anthropic's first-party extension shipped a critical
  vuln, your hand-rolled one will too. You'll be patching DOM-injection
  regressions every time the targets ship a UI update.
- **Loses signature** at the moment of paste — recipients see plaintext
  markdown. You can include the signature in the message ("here's the
  COSE signature, ask me to verify via tool") but it's vibes, not
  verification.
- Install + permission grant is a meaningful friction (a few percent of
  visitors will accept "read & change all data on chatgpt.com").

**Recommendation:** Don't ship in phase 1. If you do later, scope
permissions to specific origins and leverage the Web Extension
Manifest's `protocol_handlers` (Igalia + IPFS work landed in Chromium in
2026) to register `mnemonic://` cleanly.

### 3.6 Bookmarklet / `?q=` Prefilled URL

- ChatGPT: `https://chatgpt.com/?q=<url-encoded-prompt>` works on web
  (limited on the mobile app); ~6,000 character cap. Native param,
  undocumented but stable since 2023 per community reports.
- Claude.ai: `claude.ai/new?q=...` was officially supported, then
  **removed in October 2025** following the "Claudy Day"
  prompt-injection chain (Oasis Security disclosure). Don't rely on it.
- Gemini: `gemini.google.com/app?q=...` works.
- Perplexity, Grok: also accept `?q=`.

**Pros:** One click, no install. **Cons:** Truncation (your protocol
bundle is bigger than 6k chars unless drastically summarized), Claude.ai
removed it, signature is dropped. A hybrid works: send a short markdown
summary + a verifiable URL pointing back to the full attested bundle,
and prompt the model to fetch it via its built-in URL tool. This
degrades gracefully — the model will pull the URL on platforms that
support fetching.

### 3.7 Web Share Target API + PWA

Lets *your* webapp be a recipient of OS share-sheet content. The reverse
— sharing *from* your webapp into chatgpt.com — is not in any web
standard. Don't pursue.

### 3.8 Clipboard Handoff ("Copy as prompt")

Universal; every chat app accepts pasted markdown. Two clicks (Copy →
paste in target). Add a "Open ChatGPT / Claude / Perplexity" sibling
button that opens the destination in a new tab. Compose a self-contained
markdown system prompt that:

1. Identifies itself as a Mnemonic Protocol attested context bundle.
2. Embeds the COSE Ed25519 signature, Solana SPL Memo signature, and
   Arweave URL as verifiable references.
3. Instructs the assistant: "If you have a `verify` tool from
   `mcp.mnemonic.dev`, call it. Otherwise treat the following knowledge
   as authoritative."

**Signature preserved?** Cryptographically: no, it's just text.
*Verifiable*: yes — anyone can later re-fetch the Arweave object and
validate.

### 3.9 `registerProtocolHandler` (`mnemonic://...`)

Browser API exists in Chrome/Edge/Firefox/Opera (~89% desktop), 0% on
mobile. Names must be `web+mnemonic` (or one of an allow-listed prefix).
You can only pass text. Realistic use: register `web+mnemonic` to open
*your* webapp with a bundle ID, then the webapp redirects to the
appropriate destination. Marginal value vs. just hosting a
`mnemonic.dev/b/<id>` URL. Skip.

### 3.10 Shareable Permalink + Hosted Landing Page

Per-bundle URL like `mnemonic.dev/b/<bundle-id>` that:

1. Renders a verification UI (signature OK ✓, anchored on Solana tx ✓,
   Arweave URL).
2. Offers per-destination buttons:
   - "Open in Cursor" → `cursor://...` deeplink
   - "Open in VS Code" → `vscode:mcp/install/...`
   - "Open in Claude Desktop" → download `.mcpb` or copy MCP URL
   - "Open in ChatGPT" → copy-as-prompt + `chatgpt.com/?q=<short-prompt-with-bundle-link>`
   - "Open in Claude.ai" → copy-as-prompt + open
   - "Add to Claude.ai connectors" → instructions modal with the MCP URL

This page is your single canonical surface for *every* mechanism. Cheap
to build, big UX win. The download button becomes one option of many on
this page rather than the entire UX.

### 3.11 Cryptographic Verification — Mechanism Crosswalk

| Mechanism | Bundle bytes traverse intact? | Signature can be checked client-side? |
|---|---|---|
| Hosted MCP `recall` / `verify` | Yes | Yes (model can call `verify` tool) |
| `.mcpb` desktop extension | Yes | Yes (local Rust verifies) |
| Apps SDK app | Yes | Yes |
| Custom GPT Action | Yes | Yes via tool call |
| Bookmarklet `?q=` | No (truncated/text) | Only by URL fetch back to your origin |
| Clipboard paste | No (text only) | Only via reference to Arweave URL |
| Browser extension | Depends — can paste raw CBOR base64, but model can't verify | Only via tool callback |
| Permalink page + tool | Yes (artifact stays on Arweave) | Yes if model fetches |

The MCP path is the only one where a fresh chat in another app can both
*receive* and *cryptographically verify* the bundle inside its own
runtime. That's the protocol's differentiator — lean into it.

## 4. Recommended Phased Rollout

### Phase 1 — This month (highest reach per dev-day)

1. **Convert `mnemonic-mcp` to streamable HTTP + OAuth 2.1**, deploy at
   `mcp.mnemonic.dev`. Single hosted endpoint.
2. **Build the per-bundle landing page** `mnemonic.dev/b/<id>` with five
   buttons:
   - Cursor deeplink (one-click)
   - VS Code deeplink (one-click)
   - "Add to Claude.ai" modal (paste URL into Settings → Connectors)
   - "Add to ChatGPT (Business/Enterprise)" modal
   - "Copy as prompt + open" for ChatGPT, Claude.ai, Perplexity, Gemini
3. **Replace the current download button** with a "Get protocol
   knowledge" CTA that lands on this page. Keep file download as a
   tertiary "Advanced → Download raw bundle" option.
4. **Generate a self-contained markdown prompt template** that includes
   the Arweave URL + signature so even paste-only users get
   verifiability-by-reference.

Effort: ~2 weeks for one engineer. Reach: covers Cursor, VS Code,
Windsurf (paste URL), Claude.ai paid, Claude Desktop, Perplexity paid,
ChatGPT Business/Enterprise — likely >50% of evaluators.

### Phase 2 — Next quarter

1. **Submit to ChatGPT App Directory** via Apps SDK. Same MCP server,
   plus the optional UI layer the SDK supports. Reach jumps to ChatGPT
   Plus/Pro/Free.
2. **Publish a `.mcpb` bundle** for Claude Desktop users who want
   local-only operation (and as a Trojan horse for the `mnemonic-core`
   Rust crate — your `.mcpb` ships the binary).
3. **Apply for Claude Connectors Directory listing** (partner pipeline;
   no public portal yet, but it's worth contacting Anthropic).
4. **Gemini CLI extension** for the dev audience (cheap — wrap the same
   MCP server).

### Phase 3 — Opportunistic

1. **Browser extension** only if metrics show meaningful demand from
   ChatGPT Free users and Apps SDK approval is delayed. Treat as a
   stopgap. Strict origin allowlists. Open-source and audit it; the
   ShadowPrompt incident set the bar high.
2. **Mobile share-sheet integrations** via native iOS/Android apps if/
   when ChatGPT and Claude mobile add MCP support (not GA as of April
   2026 — Claude.ai mobile uses Anthropic's cloud connector path;
   ChatGPT mobile does not honor `?q=`).
3. **`web+mnemonic://` protocol handler** as a polished UX on top of the
   landing page once you have user volume.

## 5. Open Questions

1. **ChatGPT Plus tier MCP gating.** OpenAI's docs state full MCP /
   Developer Mode is Business/Enterprise/Edu, with Plus/Pro able to
   *use* admin-published connectors. The exact behavior for an
   individual Plus user installing a third-party Apps-SDK-published app
   (post-approval) is fuzzy across sources — verify with a Plus account
   before promising "works in Plus" in marketing.
2. **Apps SDK review SLA.** OpenAI explicitly says "we cannot offer
   estimated review times." Plan for weeks, not days. Confirm whether
   crypto/Solana surface area triggers extra scrutiny — talk to OpenAI
   DevRel before submission.
3. **Anthropic IP allowlist.** Verify your hosting provider isn't
   blocked. Anthropic publishes the IP ranges that originate connector
   traffic; double-check your WAF/Cloudflare rules.
4. **Claude.ai Free user limit.** "One custom connector slot" — confirm
   whether that's literal (1 across all time) or rotating. May affect
   how you message free-tier users.
5. **Whether `chatgpt.com/?q=` will be deprecated.** Claude.ai removed
   `claude.ai/new?q=` in Oct 2025 after a prompt-injection disclosure;
   OpenAI may follow. Don't bet long-term on URL-prefilling.
6. **MCP signature passthrough.** Verify that streamable HTTP doesn't
   re-encode CBOR in ways that invalidate the COSE signature; round-
   trip-test through Anthropic's connector proxy and OpenAI's MCP
   transport.
7. **Cursor deeplink trust UI.** Post-CursorJack, Cursor may add origin-
   verification steps that change the one-click count to two clicks.
   Re-test before phase 1 launch.
8. **Whether to publish to `mcp.directory` / Smithery / Glama.** These
   are the discovery layer; a Smithery listing is likely the second-
   highest-leverage thing after the landing page. Cost: a
   `smithery.yaml`. Worth phase 1.

## Sources

- [MCP and Connectors | OpenAI API](https://developers.openai.com/api/docs/guides/tools-connectors-mcp)
- [Building MCP servers for ChatGPT Apps and API integrations](https://developers.openai.com/api/docs/mcp)
- [Developer mode, and MCP apps in ChatGPT [beta]](https://help.openai.com/en/articles/12584461-developer-mode-apps-and-full-mcp-connectors-in-chatgpt-beta)
- [OpenAI Adds Full MCP Support to ChatGPT Developer Mode (InfoQ)](https://www.infoq.com/news/2025/10/chat-gpt-mcp/)
- [Apps SDK | OpenAI Developers](https://developers.openai.com/apps-sdk)
- [Submit and maintain your app — Apps SDK](https://developers.openai.com/apps-sdk/deploy/submission)
- [App submission guidelines — Apps SDK](https://developers.openai.com/apps-sdk/app-submission-guidelines)
- [Submitting apps to the ChatGPT app directory](https://help.openai.com/en/articles/20001040-submitting-apps-to-the-chatgpt-app-directory)
- [Developers can now submit apps to ChatGPT (OpenAI)](https://openai.com/index/developers-can-now-submit-apps-to-chatgpt/)
- [Introducing apps in ChatGPT and the new Apps SDK (OpenAI)](https://openai.com/index/introducing-apps-in-chatgpt/)
- [GPT Actions | OpenAI Platform](https://platform.openai.com/docs/actions/introduction)
- [GPT Action authentication | OpenAI API](https://platform.openai.com/docs/actions/authentication)
- [ChatGPT Shared Links FAQ](https://help.openai.com/en/articles/7925741-chatgpt-shared-links-faq)
- [ChatGPT File Upload Limit 2026 (Fastio)](https://fast.io/resources/chatgpt-file-upload-limit/)
- [Get started with custom connectors using remote MCP (Claude)](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)
- [Build custom connectors via remote MCP servers (Claude)](https://support.claude.com/en/articles/11503834-build-custom-connectors-via-remote-mcp-servers)
- [Pre-built web connectors using remote MCP](https://support.claude.com/en/articles/11176164-pre-built-web-connectors-using-remote-mcp)
- [Claude AI Connectors: One-Click Tool Integrations (2026)](https://max-productive.ai/blog/claude-ai-connectors-guide-2025/)
- [What are Claude projects?](https://support.claude.com/en/articles/9517075-what-are-projects)
- [GitHub: modelcontextprotocol/mcpb](https://github.com/modelcontextprotocol/mcpb)
- [One-click MCP server installation for Claude Desktop](https://www.anthropic.com/engineering/desktop-extensions)
- [Building Desktop Extensions with MCPB](https://support.claude.com/en/articles/12922929-building-desktop-extensions-with-mcpb)
- [Adopting the MCP Bundle format (.mcpb)](https://blog.modelcontextprotocol.io/posts/2025-11-20-adopting-mcpb/)
- [Open Claude Desktop with a link](https://support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link)
- [MCP Install Links | Cursor Docs](https://cursor.com/docs/context/mcp/install-links)
- [Model Context Protocol (MCP) | Cursor Docs](https://cursor.com/docs/context/mcp)
- [One-Click MCP Install with Cursor Deeplinks](https://aiengineerguide.com/til/cursor-mcp-deeplink/)
- [CursorJack: weaponizing Deeplinks to exploit Cursor IDE (Proofpoint)](https://www.proofpoint.com/us/blog/threat-insight/cursorjack-weaponizing-deeplinks-exploit-cursor-ide)
- [Add and manage MCP servers in VS Code](https://code.visualstudio.com/docs/copilot/customization/mcp-servers)
- [Local and Remote MCPs for Perplexity](https://www.perplexity.ai/help-center/en/articles/11502712-local-and-remote-mcps-for-perplexity)
- [Perplexity MCP Server](https://docs.perplexity.ai/docs/getting-started/integrations/mcp-server)
- [Gemini CLI extensions](https://geminicli.com/docs/extensions/)
- [Gemini CLI extensions let you customize your command line (Google)](https://blog.google/innovation-and-ai/technology/developers-tools/gemini-cli-extensions/)
- [Create custom connector | Gemini Enterprise](https://docs.cloud.google.com/gemini/enterprise/docs/connectors/create-custom-connector)
- [Tips for creating custom Gems](https://support.google.com/gemini/answer/15235603)
- [Navigator: registerProtocolHandler() (MDN)](https://developer.mozilla.org/en-US/docs/Web/API/Navigator/registerProtocolHandler)
- [Custom protocol handling | Can I use](https://caniuse.com/registerprotocolhandler)
- [Protocol Handler Registration via Browser Extensions (Igalia)](https://blogs.igalia.com/jfernandez/2026/03/24/protocol-handler-registration-via-browser-extensions/)
- [How to Run a Prompt Through a URL in ChatGPT, Perplexity, Gemini...](https://linkmyprompt.com/how-to-run-a-prompt-through-a-url-in-chatgpt-perplexity-grok-gemini-claude/)
- [URL parameters for Claude Code on the Web (issue #19023)](https://github.com/anthropics/claude-code/issues/19023)
- [Submission of prompt via URL parameter stopped working (issue #8827)](https://github.com/anthropics/claude-code/issues/8827)
- [Claude.ai Prompt Injection Vulnerability (Oasis Security)](https://www.oasis.security/blog/claude-ai-prompt-injection-data-exfiltration-vulnerability)
- [ShadowPrompt: Claude Chrome Extension Zero-Click Vulnerability (Bastion)](https://bastion.tech/blog/shadowprompt-claude-chrome-extension-vulnerability)
- [Smithery — Model Context Protocol Registry](https://smithery.ai/)
- [Best MCP Registries in 2026 (TrueFoundry)](https://www.truefoundry.com/blog/best-mcp-registries)
- [Where to Find MCP Servers in 2026 (Automation Switch)](https://automationswitch.com/ai-workflows/where-to-find-mcp-servers-2026)
